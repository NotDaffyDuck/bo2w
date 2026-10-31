//! The things standing in the world, as a client draws and clips against them: entity states,
//! script movers, dead players' bodies and the item tables that go with dropped guns.

use entity_t6::{EntitySnapshot, LerpMissile, PlayerAnimState, Trajectory};
use playerstate_t6::Weapon;
use sim::{
    DroppedItemAmmo, ItemPickupRecord, PlayerCorpsePool, PlayerCorpseSlot, MoverSlot,
};

use super::codec::{Wire, refuse};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

wire_fields!(Trajectory { began_at, tr_type, span_ms, rate, base_point });

/// The slice of an entity state a client reads. The held weapon has its own field, apart from
/// `index` (which names a brush mover's model).
impl Wire for EntitySnapshot {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.slot_index.put(wr);
        (self.e_type as u16).put(wr);
        self.lerp.e_flags.put(wr);
        self.lerp.pos.put(wr);
        self.lerp.apos.put(wr);
        (self.index as u16).put(wr);
        self.weapon.0.put(wr);
        // A brush mover's `solid` names the model the client's prediction clips against.
        self.solid.put(wr);
        self.lerp.u.missile().spawn_moment.put(wr);
        self.client_num.put(wr);
        let anim = self.un2.player_anim();
        anim.legs_clip.put(wr);
        anim.torso_clip.put(wr);
        (self.ground_entity_num as u16).put(wr);
        (self.event_sequence as u16).put(wr);
        self.events.put(wr);
        self.event_parms.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let number = i32::take(rd)?;
        let e_type = u16::take(rd)? as i16;
        let e_flags = i32::take(rd)?;
        let pos = Trajectory::take(rd)?;
        let apos = Trajectory::take(rd)?;
        let index = u16::take(rd)? as i16;
        let weapon = Weapon(u32::take(rd)?);
        let solid = i32::take(rd)?;
        let spawn_moment = i32::take(rd)?;
        let client_num_val = u8::take(rd)?;
        let legs_anim = i32::take(rd)?;
        let torso_anim_val = i32::take(rd)?;
        let ground_entity_num = u16::take(rd)? as i16;
        let event_sequence = u16::take(rd)? as i16;
        let events = <[u8; 4]>::take(rd)?;
        let event_parms = <[u32; 4]>::take(rd)?;

        let mut state = Self {
            slot_index: number,
            e_type,
            index,
            weapon,
            solid,
            client_num: client_num_val,
            ground_entity_num,
            event_sequence,
            events,
            event_parms,
            ..Self::default()
        };
        state.lerp.e_flags = e_flags;
        state.lerp.pos = pos;
        state.lerp.apos = apos;
        state.lerp.u.set_missile(&LerpMissile { spawn_moment, ..LerpMissile::default() });
        state.un2.set_player_anim(&PlayerAnimState {
            legs_clip: legs_anim,
            torso_clip: torso_anim_val,
            ..PlayerAnimState::default()
        });
        Ok(state)
    }
}

impl Wire for MoverSlot {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.id.put(wr);
        self.nonsolid.put(wr);
        self.state.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let id = Wire::take(rd)?;
        let nonsolid = bool::take(rd)?;
        let state = EntitySnapshot::take(rd)?;
        Ok(Self { id, state, nonsolid, ..Self::default() })
    }
}

/// An empty corpse slot is a lone 0; a body is a 1 and then everything that places and poses it.
impl Wire for PlayerCorpseSlot {
    fn put(&self, wr: &mut ByteSinkWriter) {
        if !self.occupied {
            wr.put_u8(0);
            return;
        }
        wr.put_u8(1);
        self.victim.put(wr);
        self.origin.put(wr);
        self.viewangles.put(wr);
        self.anim.legs.put(wr);
        self.anim.torso.put(wr);
        self.view_height_current.put(wr);
        self.weapon.put(wr);
        self.e_flags.put(wr);
        self.entnum.put(wr);
        self.tr_type.put(wr);
        self.tr_time.put(wr);
        self.tr_duration.put(wr);
        self.tr_delta.put(wr);
        self.tr_base.put(wr);
        self.falling.put(wr);
        self.ground_entity_num.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match rd.get_u8()? {
            0 => return Ok(Self::default()),
            1 => {}
            _ => return refuse("bad corpse slot tag"),
        }
        let victim = Wire::take(rd)?;
        let origin = Wire::take(rd)?;
        let viewangles_val = Wire::take(rd)?;
        let legs_anim = i32::take(rd)?;
        let torso = i32::take(rd)?;
        let view_height_current_val = f32::take(rd)?;
        let weapon = u32::take(rd)?;
        let e_flags = u32::take(rd)?;
        let entnum = i32::take(rd)?;
        let tr_type = u8::take(rd)?;
        let tr_time = i32::take(rd)?;
        let tr_duration = i32::take(rd)?;
        let tr_delta = Wire::take(rd)?;
        let tr_base = Wire::take(rd)?;
        let falling = bool::take(rd)?;
        let ground_entity_num = i32::take(rd)?;
        Ok(Self {
            occupied: true,
            entnum,
            victim,
            origin,
            viewangles: viewangles_val,
            anim: playerstate_t6::game::BodyAnims { legs: legs_anim, torso },
            view_height_current: view_height_current_val,
            weapon,
            e_flags,
            tr_type,
            tr_time,
            tr_duration,
            tr_delta,
            tr_base,
            falling,
            ground_entity_num,
        })
    }
}

/// Where the next body goes, then every slot of the ring in order.
impl Wire for PlayerCorpsePool {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.spawn_ring.put(wr);
        for cell in &self.slots {
            cell.put(wr);
        }
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let mut pool = Self { spawn_ring: u8::take(rd)?, ..Self::default() };
        for cell in &mut pool.slots {
            *cell = PlayerCorpseSlot::take(rd)?;
        }
        Ok(pool)
    }
}

wire_fields!(DroppedItemAmmo {
    entnum,
    clip_r,
    clip_l,
    stock,
    stock2,
    scavenger,
    owner_clear_ms,
});

wire_fields!(ItemPickupRecord {
    picker,
    weapon,
    from_entnum,
    clip_r,
    clip_l,
    stock,
    swapped_entnum,
    picker_pm_type,
    ammo_only,
});
