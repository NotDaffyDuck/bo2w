//! What a client asks the server to do, one tag byte and then the request's own fields.

use sim::{PlayerIntent, ClientId, PersonalClass, SpawnPick};

use super::codec::{Wire, put_list, refuse, take_list};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

wire_fields!(PersonalClass { weapons, perks });

const SEEDED_SPAWN: u8 = 0;
const SPAWN_AT: u8 = 1;

impl Wire for SpawnPick {
    fn put(&self, wr: &mut ByteSinkWriter) {
        match *self {
            Self::Seeded(seed) => {
                wr.put_u8(SEEDED_SPAWN);
                seed.put(wr);
            }
            Self::At { origin, yaw } => {
                wr.put_u8(SPAWN_AT);
                origin.put(wr);
                yaw.put(wr);
            }
        }
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match rd.get_u8()? {
            SEEDED_SPAWN => Ok(Self::Seeded(u64::take(rd)?)),
            SPAWN_AT => {
                let origin = <[f32; 3]>::take(rd)?;
                Ok(Self::At { origin, yaw: f32::take(rd)? })
            }
            _ => refuse("unknown SpawnPick tag"),
        }
    }
}

wire_variants! {
    PlayerIntent, "unknown ClientAction tag", {
        1 => PickLoadout { request_id, class_id, revision, loadout },
        2 => JoinMatch { request_id },
        3 => LeaveMatch { request_id },
        4 => ForceDeath { request_id },
        5 => StageMatch { request_id, phase },
        6 => HandOutGun { request_id, gun },
        7 => Move { request_id, origin, angles },
        8 => SpawnClient { request_id },
        9 => SpinMover { request_id, speed },
        10 => TestHarm { request_id, amount },
        11 => SetName { request_id, name },
        12 => SpawnIntermission { request_id },
        13 => UseCopycat { request_id },
        14 => ForceSpawn { request_id, pick },
        15 => ChangeWeaponConfiguration { request_id, from, to },
        16 => ChooseDefaultClass { request_id, index },
        17 => MenuResponse { request_id, menu, response },
        18 => GiveKillstreak { request_id, name },
        19 => ActionSlot { request_id, slot },
    }
}

pub(crate) fn pack_command(wr: &mut ByteSinkWriter, action: &PlayerIntent) {
    action.put(wr);
}

pub(crate) fn read_intent(rd: &mut ByteCursorReader<'_>) -> Result<PlayerIntent, PacketDecodeFault> {
    PlayerIntent::take(rd)
}

pub fn write_intent_list(wr: &mut ByteSinkWriter, list: &[(ClientId, PlayerIntent)]) {
    put_list(wr, list);
}

pub fn read_intent_list(rd: &mut ByteCursorReader<'_>) -> Result<Vec<(ClientId, PlayerIntent)>, PacketDecodeFault> {
    take_list(rd, 64)
}
