//! What a client asks the server to do, one tag byte and then the request's own fields.

use sim::{ClientAction, ClientId, PersonalClass, SpawnPick};

use super::codec::{Decoded, Wire, put_list, refuse, take_list};
use crate::transport::wire::{WireReader, WireWriter};

wire_fields!(PersonalClass { weapons, perks, deathstreak });

const SEEDED_SPAWN: u8 = 0;
const SPAWN_AT: u8 = 1;

impl Wire for SpawnPick {
    fn put(&self, out: &mut WireWriter) {
        match *self {
            Self::Seeded(seed) => {
                out.put_u8(SEEDED_SPAWN);
                seed.put(out);
            }
            Self::At { origin, yaw } => {
                out.put_u8(SPAWN_AT);
                origin.put(out);
                yaw.put(out);
            }
        }
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match input.get_u8()? {
            SEEDED_SPAWN => Ok(Self::Seeded(u64::take(input)?)),
            SPAWN_AT => {
                let origin = <[f32; 3]>::take(input)?;
                Ok(Self::At { origin, yaw: f32::take(input)? })
            }
            _ => refuse("unknown SpawnPick tag"),
        }
    }
}

wire_variants! {
    ClientAction, "unknown ClientAction tag", {
        1 => SelectClass { request_id, class_id, revision, loadout },
        2 => JoinMatch { request_id },
        3 => LeaveMatch { request_id },
        4 => ForceDeath { request_id },
        5 => SetMatchPhase { request_id, phase },
        6 => GiveWeapon { request_id, weapon },
        7 => Move { request_id, origin, angles },
        8 => SpawnClient { request_id },
        9 => BeginScriptMoverRotateVelocity { request_id, speed },
        10 => DebugDamage { request_id, amount },
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

pub(crate) fn encode_action(out: &mut WireWriter, action: &ClientAction) {
    action.put(out);
}

pub(crate) fn decode_action(input: &mut WireReader<'_>) -> Decoded<ClientAction> {
    ClientAction::take(input)
}

pub fn encode_actions(out: &mut WireWriter, actions: &[(ClientId, ClientAction)]) {
    put_list(out, actions);
}

pub fn decode_actions(input: &mut WireReader<'_>) -> Decoded<Vec<(ClientId, ClientAction)>> {
    take_list(input, 64)
}
