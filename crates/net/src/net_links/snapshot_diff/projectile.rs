//! Projectiles on the wire: a count of changed projectiles with their full state, then a count of
//! removed ids. A projectile that did not change since the baseline is not mentioned.

use sim::{ClientId, LifeSequence, MissileGuide, MissileTarget, ProjectileId, FlyingProjectile};
use std::collections::{HashMap, HashSet};

use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectileChangeRecord {
    pub changed: Vec<FlyingProjectile>,
    pub gone: Vec<ProjectileId>,
}

/// The wire value for "no detonation time".
const NO_DETONATION: i32 = i32::MIN;

/// The wire value for "no bone" on a stuck rider.
const NO_BONE: u16 = u16::MAX;

fn put_triple(sink: &mut ByteSinkWriter, values: [f32; 3]) {
    values.into_iter().for_each(|axis| sink.write_real32(axis));
}

/// A list length; lists never outgrow a u16.
fn put_count(sink: &mut ByteSinkWriter, length: usize) {
    debug_assert!(u16::try_from(length).is_ok());
    sink.put_u16(length as u16);
}

fn get_triple(cursor: &mut ByteCursorReader<'_>) -> Result<[f32; 3], PacketDecodeFault> {
    Ok([cursor.read_f32()?, cursor.read_f32()?, cursor.read_f32()?])
}

fn put_trajectory(sink: &mut ByteSinkWriter, path: &entity_t6::Trajectory) {
    sink.poke_int32(path.began_at);
    sink.put_u8(path.tr_type);
    sink.poke_int32(path.span_ms);
    put_triple(sink, path.rate);
    put_triple(sink, path.base_point);
}

fn get_trajectory(cursor: &mut ByteCursorReader<'_>) -> Result<entity_t6::Trajectory, PacketDecodeFault> {
    let tr_time = cursor.get_i32()?;
    let tr_type = cursor.get_u8()?;
    let tr_duration = cursor.get_i32()?;
    let tr_delta = get_triple(cursor)?;
    let tr_base = get_triple(cursor)?;
    Ok(entity_t6::Trajectory {
        began_at: tr_time,
        tr_type,
        span_ms: tr_duration,
        rate: tr_delta,
        base_point: tr_base,
    })
}

/// A missile's target: a tag (0 none, 1 point, 2 entity, 3 player), the identity that tag needs,
/// then the offset.
fn put_target(sink: &mut ByteSinkWriter, target: Option<MissileTarget>) {
    let Some(target) = target else {
        sink.put_u8(0);
        return;
    };
    // Entities send number then generation, players client then life (all as 32-bit words).
    let (tag, identity, offset) = match target {
        MissileTarget::Point(point) => (1, None, point),
        MissileTarget::Entity { entity, offset } => (2, Some((entity.slot() as u32, entity.generation())), offset),
        MissileTarget::Player { client, life, offset } => (3, Some((client.0, life.0)), offset),
    };
    sink.put_u8(tag);
    if let Some((id_word, life_word)) = identity {
        sink.put_u32(id_word);
        sink.put_u32(life_word);
    }
    put_triple(sink, offset);
}

fn get_target(cursor: &mut ByteCursorReader<'_>) -> Result<Option<MissileTarget>, PacketDecodeFault> {
    let kind = cursor.get_u8()?;
    if kind == 0 {
        return Ok(None);
    }
    if kind > 3 {
        return Err(PacketDecodeFault::GarbledRequest("bad missile target tag"));
    }
    let identity = if kind == 1 { None } else { Some((cursor.read_word32()?, cursor.read_word32()?)) };
    let offset = get_triple(cursor)?;
    Ok(Some(match identity {
        None => MissileTarget::Point(offset),
        Some((slot_idx, era)) if kind == 2 => {
            let entity = sim::EntHandle::from_parts(slot_idx as i32, era)
                .map_err(|_| PacketDecodeFault::GarbledRequest("bad missile target entity"))?;
            MissileTarget::Entity { entity, offset }
        }
        Some((who, lives)) => MissileTarget::Player { client: ClientId(who), life: LifeSequence(lives), offset },
    }))
}

fn put_projectile(sink: &mut ByteSinkWriter, p: &FlyingProjectile) {
    sink.put_u32(p.id.0);
    sink.put_u32(p.owner.0);
    sink.put_u32(p.owner_respawn_count.0);
    sink.put_u32(p.gun);
    put_triple(sink, p.origin);
    put_triple(sink, p.motion);
    sink.poke_int32(p.spawn_time_ms);
    sink.poke_int32(p.detonate_at_ms.unwrap_or(NO_DETONATION));
    sink.poke_int32(p.cleanup_at_ms);
    sink.write_real32(p.travel_distance);
    sink.put_u32(u32::from(p.in_flight));
    put_trajectory(sink, &p.at);
    put_trajectory(sink, &p.turn_path);
    sink.poke_int32(p.ent_no);
    sink.poke_int32(p.spawn_moment);
    sink.put_u32(p.stuck_pane.map_or(0, |pane| pane.saturating_add(1)));
    sink.put_u32(u32::from(p.grounded));

    let guide = &p.guide;
    put_target(sink, guide.target);
    sink.put_u8(u8::from(guide.top));
    sink.put_u8(guide.stage);
    sink.put_u8(u8::from(guide.went_by));
    put_triple(sink, guide.offset);
    sink.write_real32(guide.bank);
    put_target(sink, guide.home);
    put_triple(sink, guide.from);

    // The player it is stuck to, so that player's own screen leaves its grenade warning off.
    match p.rider {
        Some(rider) => {
            sink.put_u8(1 | (u8::from(rider.shield) << 1));
            sink.put_u32(rider.client.0);
            sink.put_u32(rider.life.0);
            sink.put_u16(rider.bone.unwrap_or(NO_BONE));
            put_triple(sink, rider.local);
        }
        None => sink.put_u8(0),
    }
}

fn get_projectile(cursor: &mut ByteCursorReader<'_>) -> Result<FlyingProjectile, PacketDecodeFault> {
    let id = ProjectileId(cursor.read_word32()?);
    let owner = ClientId(cursor.read_word32()?);
    let owner_respawn_count = LifeSequence(cursor.read_word32()?);
    let weapon = cursor.read_word32()?;
    let origin = get_triple(cursor)?;
    let velocity = get_triple(cursor)?;
    let spawn_time_ms = cursor.get_i32()?;
    let detonate_at_ms = Some(cursor.get_i32()?).filter(|&ms| ms != NO_DETONATION);
    let cleanup_at_ms = cursor.get_i32()?;
    let travel_distance = cursor.read_f32()?;
    let live = cursor.read_word32()? != 0;
    let pos = get_trajectory(cursor)?;
    let apos = get_trajectory(cursor)?;
    let entnum = cursor.get_i32()?;
    let spawn_moment = cursor.get_i32()?;
    let stuck_pane = Some(cursor.read_word32()?)
        .filter(|&encoded| encoded != 0)
        .map(|encoded| encoded - 1);
    let grounded = cursor.read_word32()? != 0;

    let target = get_target(cursor)?;
    let top = cursor.get_u8()? != 0;
    let stage = cursor.get_u8()?;
    let passed = cursor.get_u8()? != 0;
    let offset = get_triple(cursor)?;
    let roll = cursor.read_f32()?;
    let home = get_target(cursor)?;
    let from = get_triple(cursor)?;

    let flags = cursor.get_u8()?;
    let rider = if flags & 1 != 0 {
        let client = ClientId(cursor.read_word32()?);
        let life = LifeSequence(cursor.read_word32()?);
        let bone = Some(cursor.get_u16()?).filter(|&bone| bone != NO_BONE);
        let local = get_triple(cursor)?;
        Some(sim::StuckRider {
            client,
            life,
            bone,
            local,
            shield: flags & 2 != 0,
        })
    } else {
        None
    };

    Ok(FlyingProjectile {
        id,
        owner,
        owner_respawn_count,
        gun: weapon,
        origin,
        motion: velocity,
        spawn_time_ms,
        detonate_at_ms,
        cleanup_at_ms,
        travel_distance,
        in_flight: live,
        at: pos,
        turn_path: apos,
        ent_no: entnum,
        spawn_moment,
        stuck_pane,
        grounded,
        guide: MissileGuide {
            target,
            top,
            stage,
            went_by: passed,
            offset,
            bank: roll,
            home,
            from,
        },
        // Only the server knows whom its last impact landed on.
        impact_target: None,
        rider,
        // Only the server knows it flies on without a blast (`setmissilecoasting`).
        coasting: false,
    })
}

/// Write the projectiles that differ from `agreed` and the ids that left it; returns what was
/// written, in id order.
pub(super) fn encode_projectiles(
    sink: &mut ByteSinkWriter,
    agreed: &HashMap<ProjectileId, FlyingProjectile>,
    current: &[FlyingProjectile],
) -> ProjectileChangeRecord {
    let mut changed: Vec<FlyingProjectile> = current
        .iter()
        .filter(|projectile| agreed.get(&projectile.id) != Some(projectile))
        .copied()
        .collect();
    changed.sort_by_key(|projectile| projectile.id.0);

    let still_present: HashSet<ProjectileId> = current.iter().map(|p| p.id).collect();
    let mut gone: Vec<ProjectileId> = agreed.keys().copied().filter(|id| !still_present.contains(id)).collect();
    // Ids are unique, so the order is the same whichever sort runs.
    gone.sort_unstable_by_key(|id| id.0);

    put_count(sink, changed.len());
    changed.iter().for_each(|flying| put_projectile(sink, flying));
    put_count(sink, gone.len());
    gone.iter().for_each(|id| sink.put_u32(id.0));
    ProjectileChangeRecord { changed, gone: gone }
}

/// Read what [`encode_projectiles`] wrote and apply it to `agreed`.
pub(super) fn decode_projectiles(
    cursor: &mut ByteCursorReader<'_>,
    agreed: &mut HashMap<ProjectileId, FlyingProjectile>,
) -> Result<ProjectileChangeRecord, PacketDecodeFault> {
    let modified_entry_total = cursor.get_u16()? as usize;
    let mut changed = Vec::with_capacity(modified_entry_total.min(256));
    for _ in 0..modified_entry_total {
        let fresh = get_projectile(cursor)?;
        changed.push(fresh);
        agreed.insert(fresh.id, fresh);
    }
    let deleted_entry_total = cursor.get_u16()? as usize;
    let mut dropped_ids = Vec::with_capacity(deleted_entry_total.min(256));
    for _ in 0..deleted_entry_total {
        let gone = ProjectileId(cursor.read_word32()?);
        dropped_ids.push(gone);
        agreed.remove(&gone);
    }
    Ok(ProjectileChangeRecord { changed, gone: dropped_ids })
}

#[cfg(test)]
pub(super) fn sample(id: u32) -> FlyingProjectile {
    FlyingProjectile {
        id: ProjectileId(id),
        owner: ClientId(3),
        owner_respawn_count: LifeSequence(2),
        gun: 41,
        origin: [1.0, 2.0, 3.0],
        motion: [0.5, 0.0, -9.0],
        spawn_time_ms: 1000,
        detonate_at_ms: Some(4000),
        cleanup_at_ms: 9000,
        travel_distance: 12.5,
        in_flight: true,
        at: entity_t6::Trajectory::default(),
        turn_path: entity_t6::Trajectory::default(),
        ent_no: 77,
        spawn_moment: 990,
        stuck_pane: Some(0),
        grounded: false,
        guide: MissileGuide::default(),
        impact_target: None,
        rider: None,
        coasting: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(agreed: &HashMap<ProjectileId, FlyingProjectile>, now: &[FlyingProjectile]) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        encode_projectiles(&mut sink, agreed, now);
        sink.as_slice().to_vec()
    }

    #[test]
    fn a_full_projectile_survives_the_wire() {
        let mut full = sample(5);
        full.detonate_at_ms = None;
        full.stuck_pane = Some(6);
        full.guide = MissileGuide {
            target: Some(MissileTarget::Point([1.0, 2.0, 3.0])),
            top: true,
            stage: 2,
            went_by: true,
            offset: [0.0, 1.0, 0.0],
            bank: 15.0,
            home: Some(MissileTarget::Player {
                client: ClientId(9),
                life: LifeSequence(4),
                offset: [0.0, 0.0, 2.0],
            }),
            from: [4.0, 5.0, 6.0],
        };
        full.rider = Some(sim::StuckRider {
            client: ClientId(8),
            life: LifeSequence(1),
            bone: Some(12),
            local: [0.25, 0.5, 0.75],
            shield: true,
        });
        let bytes = write(&HashMap::new(), &[full]);
        let mut agreed = HashMap::new();
        let delta = decode_projectiles(&mut ByteCursorReader::new(&bytes), &mut agreed).unwrap();
        assert_eq!(delta.changed, vec![full]);
        assert_eq!(agreed.get(&ProjectileId(5)), Some(&full));
    }

    #[test]
    fn server_only_fields_are_not_sent() {
        let mut server = sample(1);
        server.coasting = true;
        let bytes = write(&HashMap::new(), &[server]);
        let mut agreed = HashMap::new();
        let delta = decode_projectiles(&mut ByteCursorReader::new(&bytes), &mut agreed).unwrap();
        assert!(!delta.changed[0].coasting);
        assert!(delta.changed[0].impact_target.is_none());
    }

    #[test]
    fn an_unchanged_projectile_is_not_mentioned_and_a_vanished_one_is_removed() {
        let kept = sample(1);
        let gone = sample(2);
        let agreed: HashMap<_, _> = [(kept.id, kept), (gone.id, gone)].into_iter().collect();
        let mut sink = ByteSinkWriter::new();
        let delta = encode_projectiles(&mut sink, &agreed, &[kept]);
        assert!(delta.changed.is_empty());
        assert_eq!(delta.gone, vec![ProjectileId(2)]);
        // Two zero counts' worth of bytes plus one removed id.
        assert_eq!(sink.into_bytes().len(), 2 + 2 + 4);
    }

    #[test]
    fn changed_and_removed_lists_come_out_in_id_order() {
        let now = [sample(9), sample(3)];
        let old = sample(7);
        let agreed: HashMap<_, _> = [(old.id, old)].into_iter().collect();
        let mut sink = ByteSinkWriter::new();
        let delta = encode_projectiles(&mut sink, &agreed, &now);
        let ids: Vec<u32> = delta.changed.iter().map(|p| p.id.0).collect();
        assert_eq!(ids, vec![3, 9]);
        assert_eq!(delta.gone, vec![ProjectileId(7)]);
    }

    #[test]
    fn decoding_applies_changes_and_removals_to_the_baseline() {
        let first = sample(1);
        let second = sample(2);
        let mut agreed: HashMap<_, _> = [(second.id, second)].into_iter().collect();
        let bytes = write(&agreed, &[first]);
        decode_projectiles(&mut ByteCursorReader::new(&bytes), &mut agreed).unwrap();
        assert!(agreed.contains_key(&first.id));
        assert!(!agreed.contains_key(&second.id));
    }

    #[test]
    fn a_missile_target_round_trips_in_each_form() {
        for target in [
            None,
            Some(MissileTarget::Point([1.0, 2.0, 3.0])),
            Some(MissileTarget::Player {
                client: ClientId(5),
                life: LifeSequence(6),
                offset: [7.0, 8.0, 9.0],
            }),
        ] {
            let mut sink = ByteSinkWriter::new();
            put_target(&mut sink, target);
            let bytes = sink.into_bytes();
            let mut cursor = ByteCursorReader::new(&bytes);
            assert_eq!(get_target(&mut cursor).unwrap(), target);
            assert!(cursor.is_empty());
        }
    }

    #[test]
    fn an_unknown_target_tag_is_malformed() {
        let err = get_target(&mut ByteCursorReader::new(&[9u8])).unwrap_err();
        assert!(matches!(err, PacketDecodeFault::GarbledRequest("bad missile target tag")));
    }
}
