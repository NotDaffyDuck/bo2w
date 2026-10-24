//! The things standing in the world, as a client draws and clips against them: entity states,
//! script movers, dead players' bodies and the item tables that go with dropped guns.

use entity_t6::{EntityState, LerpMissile, PlayerAnimState, Trajectory};
use playerstate_t6::Weapon;
use sim::{
    DroppedItemAmmo, ItemPickupRecord, PlayerCorpsePool, PlayerCorpseSlot, ScriptMoverGentity,
};

use super::codec::{Decoded, Wire, refuse};
use crate::transport::wire::{WireReader, WireWriter};

wire_fields!(Trajectory { tr_time, tr_type, tr_duration, tr_delta, tr_base });

/// The slice of an entity state a client reads. The held weapon has its own field, apart from
/// `index` (which names a brush mover's model).
impl Wire for EntityState {
    fn put(&self, out: &mut WireWriter) {
        self.number.put(out);
        (self.e_type as u16).put(out);
        self.lerp.e_flags.put(out);
        self.lerp.pos.put(out);
        self.lerp.apos.put(out);
        (self.index as u16).put(out);
        self.weapon.0.put(out);
        // A brush mover's `solid` names the model the client's prediction clips against.
        self.solid.put(out);
        self.lerp.u.missile().launch_time.put(out);
        self.client_num.put(out);
        let anim = self.un2.player_anim();
        anim.legs_anim.put(out);
        anim.torso_anim.put(out);
        (self.ground_entity_num as u16).put(out);
        (self.event_sequence as u16).put(out);
        self.events.put(out);
        self.event_parms.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let number = i32::take(input)?;
        let e_type = u16::take(input)? as i16;
        let e_flags = i32::take(input)?;
        let pos = Trajectory::take(input)?;
        let apos = Trajectory::take(input)?;
        let index = u16::take(input)? as i16;
        let weapon = Weapon(u32::take(input)?);
        let solid = i32::take(input)?;
        let launch_time = i32::take(input)?;
        let client_num = u8::take(input)?;
        let legs_anim = i32::take(input)?;
        let torso_anim = i32::take(input)?;
        let ground_entity_num = u16::take(input)? as i16;
        let event_sequence = u16::take(input)? as i16;
        let events = <[u8; 4]>::take(input)?;
        let event_parms = <[u32; 4]>::take(input)?;

        let mut state = Self {
            number,
            e_type,
            index,
            weapon,
            solid,
            client_num,
            ground_entity_num,
            event_sequence,
            events,
            event_parms,
            ..Self::default()
        };
        state.lerp.e_flags = e_flags;
        state.lerp.pos = pos;
        state.lerp.apos = apos;
        state.lerp.u.set_missile(&LerpMissile { launch_time, ..LerpMissile::default() });
        state.un2.set_player_anim(&PlayerAnimState {
            legs_anim,
            torso_anim,
            ..PlayerAnimState::default()
        });
        Ok(state)
    }
}

impl Wire for ScriptMoverGentity {
    fn put(&self, out: &mut WireWriter) {
        self.id.put(out);
        self.nonsolid.put(out);
        self.state.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let id = Wire::take(input)?;
        let nonsolid = bool::take(input)?;
        let state = EntityState::take(input)?;
        Ok(Self { id, state, nonsolid, ..Self::default() })
    }
}

/// An empty corpse slot is a lone 0; a body is a 1 and then everything that places and poses it.
impl Wire for PlayerCorpseSlot {
    fn put(&self, out: &mut WireWriter) {
        if !self.occupied {
            out.put_u8(0);
            return;
        }
        out.put_u8(1);
        self.victim.put(out);
        self.origin.put(out);
        self.viewangles.put(out);
        self.anim.legs_anim.put(out);
        self.anim.torso_anim.put(out);
        self.view_height_current.put(out);
        self.weapon.put(out);
        self.e_flags.put(out);
        self.entnum.put(out);
        self.tr_type.put(out);
        self.tr_time.put(out);
        self.tr_duration.put(out);
        self.tr_delta.put(out);
        self.tr_base.put(out);
        self.falling.put(out);
        self.ground_entity_num.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match input.get_u8()? {
            0 => return Ok(Self::default()),
            1 => {}
            _ => return refuse("bad corpse slot tag"),
        }
        let victim = Wire::take(input)?;
        let origin = Wire::take(input)?;
        let viewangles = Wire::take(input)?;
        let legs_anim = i32::take(input)?;
        let torso_anim = i32::take(input)?;
        let view_height_current = f32::take(input)?;
        let weapon = u32::take(input)?;
        let e_flags = u32::take(input)?;
        let entnum = i32::take(input)?;
        let tr_type = u8::take(input)?;
        let tr_time = i32::take(input)?;
        let tr_duration = i32::take(input)?;
        let tr_delta = Wire::take(input)?;
        let tr_base = Wire::take(input)?;
        let falling = bool::take(input)?;
        let ground_entity_num = i32::take(input)?;
        Ok(Self {
            occupied: true,
            entnum,
            victim,
            origin,
            viewangles,
            anim: playerstate_t6::game::AnimPair { legs_anim, torso_anim },
            view_height_current,
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
    fn put(&self, out: &mut WireWriter) {
        self.spawn_ring.put(out);
        for slot in &self.slots {
            slot.put(out);
        }
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let mut pool = Self { spawn_ring: u8::take(input)?, ..Self::default() };
        for slot in &mut pool.slots {
            *slot = PlayerCorpseSlot::take(input)?;
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
