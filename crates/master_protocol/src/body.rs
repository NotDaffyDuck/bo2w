//! Body layouts, one per frame kind. Requests, responses and peer events start with a
//! one-byte tag naming the variant; the other kinds have a single layout.

use std::collections::HashMap;

use crate::bytes::{In, Out, Scalar};
use crate::{
    AdmissionFailure, Advert, ContentFlags, ControlHello, MemberId, PeerEvent, ProtocolError,
    RequestBody, ResponseBody, RoomView, ServiceError, MAX_ADVERT_NAME_BYTES, MAX_BUILD_BYTES,
    MAX_LIST_ADVERTS, MAX_MAP_BYTES, MAX_MODE_BYTES, MAX_PLAYER_NAME_BYTES, MAX_SESSION_MEMBERS,
};

/// Request variant tags.
mod ask {
    pub const STATUS: u8 = 1;
    pub const CREATE: u8 = 2;
    pub const JOIN: u8 = 3;
    pub const LEAVE: u8 = 4;
    pub const OPTIONS: u8 = 5;
    pub const START: u8 = 6;
    pub const END: u8 = 7;
    pub const CLOSE: u8 = 8;
    pub const LIST: u8 = 9;
    pub const WORLD_READY: u8 = 10;
    pub const MAP_LOADED: u8 = 11;
    pub const SNAPSHOT_APPLIED: u8 = 12;
    pub const ENTER: u8 = 13;
    pub const SKIP_VOTE: u8 = 14;
    pub const ADMIT_FAILED: u8 = 15;
}

/// Response variant tags. `REFUSED` sits apart so new answers can take 11 and up.
mod answer {
    pub const STATUS: u8 = 1;
    pub const CREATED: u8 = 2;
    pub const JOINED: u8 = 3;
    pub const LEFT: u8 = 4;
    pub const UPDATED: u8 = 5;
    pub const LIST: u8 = 9;
    pub const ACK: u8 = 10;
    pub const REFUSED: u8 = 0x7f;
}

/// Peer event tags.
mod relayed {
    pub const WORLD_READY: u8 = 1;
    pub const MAP_LOADED: u8 = 2;
    pub const SNAPSHOT_APPLIED: u8 = 3;
    pub const ENTER: u8 = 4;
    pub const SKIP_VOTE: u8 = 5;
    pub const ADMIT_FAILED: u8 = 6;
}

/// Text written in the room view for a member whose name the service does not hold.
const UNNAMED_MEMBER: &str = "Player";

/// The rule for what a room may be called and how big it may be. Checked when writing
/// a create request or a room view; a reader takes whatever fits the text caps.
fn check_room(name: &str, map: &str, mode: &str, max_players: u8) -> Result<(), ProtocolError> {
    let name_ok = (1..=MAX_ADVERT_NAME_BYTES).contains(&name.len());
    let players_ok = (2..=MAX_SESSION_MEMBERS).contains(&max_players);
    if name_ok && players_ok {
        check_map_mode(map, mode)
    } else {
        Err(ProtocolError::InvalidBody)
    }
}

fn check_map_mode(map: &str, mode: &str) -> Result<(), ProtocolError> {
    let map_ok = (1..=MAX_MAP_BYTES).contains(&map.len());
    let mode_ok = (1..=MAX_MODE_BYTES).contains(&mode.len());
    if map_ok && mode_ok {
        Ok(())
    } else {
        Err(ProtocolError::InvalidBody)
    }
}

impl Scalar for AdmissionFailure {
    fn emit(self, out: &mut Out) {
        let (code, digests) = match self {
            Self::Deadline => (1u8, None),
            Self::Cancelled => (2, None),
            Self::MapContent { host, peer } => (3, Some((host, peer))),
            Self::WeaponRegistry { host, peer } => (4, Some((host, peer))),
            Self::ClassRegistry { host, peer } => (5, Some((host, peer))),
        };
        out.put(code);
        if let Some((host, peer)) = digests {
            out.put(host).put(peer);
        }
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ProtocolError> {
        let code = src.get::<u8>()?;
        let pair = |src: &mut In<'_>| -> Result<(u64, u64), ProtocolError> {
            Ok((src.get()?, src.get()?))
        };
        Ok(match code {
            1 => Self::Deadline,
            2 => Self::Cancelled,
            3 => {
                let (host, peer) = pair(src)?;
                Self::MapContent { host, peer }
            }
            4 => {
                let (host, peer) = pair(src)?;
                Self::WeaponRegistry { host, peer }
            }
            5 => {
                let (host, peer) = pair(src)?;
                Self::ClassRegistry { host, peer }
            }
            _ => return Err(ProtocolError::InvalidBody),
        })
    }
}

impl Scalar for ServiceError {
    fn emit(self, out: &mut Out) {
        let code: u8 = match self {
            Self::Malformed => 1,
            Self::InvalidAdvert => 2,
            Self::AdvertCapacity => 3,
            Self::AlreadyHosting => 4,
            Self::UnknownAdvert => 5,
            Self::Full => 7,
            Self::Locked => 8,
            Self::NotHost => 11,
            Self::NotMember => 13,
            Self::MissingContent { requires, have } => {
                out.put(14u8).put(requires).put(have);
                return;
            }
        };
        out.put(code);
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ProtocolError> {
        Ok(match src.get::<u8>()? {
            1 => Self::Malformed,
            2 => Self::InvalidAdvert,
            3 => Self::AdvertCapacity,
            4 => Self::AlreadyHosting,
            5 => Self::UnknownAdvert,
            7 => Self::Full,
            8 => Self::Locked,
            11 => Self::NotHost,
            13 => Self::NotMember,
            14 => Self::MissingContent {
                requires: src.get()?,
                have: src.get()?,
            },
            _ => return Err(ProtocolError::InvalidBody),
        })
    }
}

pub(crate) fn write_hello(out: &mut Out, hello: &ControlHello) -> Result<(), ProtocolError> {
    out.put(hello.protocol_version)
        .put(hello.game_protocol)
        .put(hello.role)
        .text(&hello.build, MAX_BUILD_BYTES)?
        .text(&hello.player_name, MAX_PLAYER_NAME_BYTES)?;
    Ok(())
}

pub(crate) fn read_hello(src: &mut In<'_>) -> Result<ControlHello, ProtocolError> {
    Ok(ControlHello {
        protocol_version: src.get()?,
        game_protocol: src.get()?,
        role: src.get()?,
        build: src.text(MAX_BUILD_BYTES)?,
        player_name: src.text(MAX_PLAYER_NAME_BYTES)?,
    })
}

/// Map, mode: the pair shared by create, options and start.
fn write_map_mode(out: &mut Out, map: &str, mode: &str) -> Result<(), ProtocolError> {
    out.text(map, MAX_MAP_BYTES)?.text(mode, MAX_MODE_BYTES)?;
    Ok(())
}

/// Map, weapon-table and class-catalog digests, in that order.
fn write_digests(out: &mut Out, map: u64, weapons: u64, classes: u64) {
    out.put(map).put(weapons).put(classes);
}

pub(crate) fn write_request(out: &mut Out, body: &RequestBody) -> Result<(), ProtocolError> {
    use RequestBody as R;
    match body {
        R::Status => {
            out.put(ask::STATUS);
        }
        R::CreateRoom {
            name,
            map,
            mode,
            max_players,
            requires,
        } => {
            check_room(name, map, mode, *max_players)?;
            out.put(ask::CREATE).text(name, MAX_ADVERT_NAME_BYTES)?;
            write_map_mode(out, map, mode)?;
            out.put(*max_players).put(*requires);
        }
        R::JoinRoom { room_id, have } => {
            out.put(ask::JOIN).put(*room_id).put(*have);
        }
        R::LeaveRoom => {
            out.put(ask::LEAVE);
        }
        R::SetOptions {
            map,
            mode,
            joinable,
        } => {
            check_map_mode(map, mode)?;
            write_map_mode(out.put(ask::OPTIONS), map, mode)?;
            out.put(*joinable);
        }
        R::StartMatch { map, mode } => {
            check_map_mode(map, mode)?;
            write_map_mode(out.put(ask::START), map, mode)?;
        }
        R::EndMatch { room_id, epoch } => {
            out.put(ask::END).put(*room_id).put(*epoch);
        }
        R::CloseRoom => {
            out.put(ask::CLOSE);
        }
        R::ListRooms => {
            out.put(ask::LIST);
        }
        R::HostWorldReady {
            room_id,
            epoch,
            map,
            weapons,
            classes,
        } => {
            out.put(ask::WORLD_READY).put(*room_id).put(*epoch);
            write_digests(out, *map, *weapons, *classes);
        }
        R::MapLoaded {
            epoch,
            map,
            weapons,
            classes,
        } => {
            out.put(ask::MAP_LOADED).put(*epoch);
            write_digests(out, *map, *weapons, *classes);
        }
        R::BootstrapApplied {
            epoch,
            bootstrap_id,
            snapshot_seq,
            connection,
        } => {
            out.put(ask::SNAPSHOT_APPLIED)
                .put(*epoch)
                .put(*bootstrap_id)
                .put(*snapshot_seq)
                .put(*connection);
        }
        R::EnterMatch {
            member_id,
            epoch,
            bootstrap_id,
            connection_id,
            client_id,
        } => {
            out.put(ask::ENTER)
                .put(*member_id)
                .put(*epoch)
                .put(*bootstrap_id)
                .put(*connection_id)
                .put(*client_id);
        }
        R::VoteToSkip => {
            out.put(ask::SKIP_VOTE);
        }
        R::AdmissionFailed {
            member_id,
            epoch,
            reason,
        } => {
            out.put(ask::ADMIT_FAILED)
                .put(*member_id)
                .put(*epoch)
                .put(*reason);
        }
    }
    Ok(())
}

pub(crate) fn read_request(src: &mut In<'_>) -> Result<RequestBody, ProtocolError> {
    use RequestBody as R;
    Ok(match src.get::<u8>()? {
        ask::STATUS => R::Status,
        ask::CREATE => R::CreateRoom {
            name: src.text(MAX_ADVERT_NAME_BYTES)?,
            map: src.text(MAX_MAP_BYTES)?,
            mode: src.text(MAX_MODE_BYTES)?,
            max_players: src.get()?,
            requires: src.get()?,
        },
        ask::JOIN => R::JoinRoom {
            room_id: src.get()?,
            have: src.get()?,
        },
        ask::LEAVE => R::LeaveRoom,
        ask::OPTIONS => R::SetOptions {
            map: src.text(MAX_MAP_BYTES)?,
            mode: src.text(MAX_MODE_BYTES)?,
            joinable: src.get()?,
        },
        ask::START => R::StartMatch {
            map: src.text(MAX_MAP_BYTES)?,
            mode: src.text(MAX_MODE_BYTES)?,
        },
        ask::END => R::EndMatch {
            room_id: src.get()?,
            epoch: src.get()?,
        },
        ask::CLOSE => R::CloseRoom,
        ask::LIST => R::ListRooms,
        ask::WORLD_READY => R::HostWorldReady {
            room_id: src.get()?,
            epoch: src.get()?,
            map: src.get()?,
            weapons: src.get()?,
            classes: src.get()?,
        },
        ask::MAP_LOADED => R::MapLoaded {
            epoch: src.get()?,
            map: src.get()?,
            weapons: src.get()?,
            classes: src.get()?,
        },
        ask::SNAPSHOT_APPLIED => R::BootstrapApplied {
            epoch: src.get()?,
            bootstrap_id: src.get()?,
            snapshot_seq: src.get()?,
            connection: src.get()?,
        },
        ask::ENTER => R::EnterMatch {
            member_id: src.get()?,
            epoch: src.get()?,
            bootstrap_id: src.get()?,
            connection_id: src.get()?,
            client_id: src.get()?,
        },
        ask::SKIP_VOTE => R::VoteToSkip,
        ask::ADMIT_FAILED => R::AdmissionFailed {
            member_id: src.get()?,
            epoch: src.get()?,
            reason: src.get()?,
        },
        other => return Err(ProtocolError::UnknownTag(other)),
    })
}

pub(crate) fn write_view(out: &mut Out, view: &RoomView) -> Result<(), ProtocolError> {
    let count = u8::try_from(view.members.len())
        .ok()
        .filter(|&count| count <= MAX_SESSION_MEMBERS)
        .ok_or(ProtocolError::LengthExceeded(view.members.len()))?;
    check_room(&view.name, &view.map, &view.mode, view.max_players)?;
    out.put(view.room_id)
        .put(view.host)
        .put(view.revision)
        .put(view.epoch)
        .put(view.phase)
        .put(view.joinable)
        .put(view.max_players)
        .put(view.requires)
        .text(&view.name, MAX_ADVERT_NAME_BYTES)?;
    write_map_mode(out, &view.map, &view.mode)?;
    out.put(count);
    for &member in &view.members {
        let shown = view
            .member_names
            .get(&member)
            .map_or(UNNAMED_MEMBER, String::as_str);
        out.put(member).text(shown, MAX_PLAYER_NAME_BYTES)?;
    }
    Ok(())
}

pub(crate) fn read_view(src: &mut In<'_>) -> Result<RoomView, ProtocolError> {
    let room_id = src.get()?;
    let host = src.get()?;
    let revision = src.get()?;
    let epoch = src.get()?;
    let phase = src.get()?;
    let joinable = src.get()?;
    let max_players = src.get()?;
    let requires = src.get()?;
    let name = src.text(MAX_ADVERT_NAME_BYTES)?;
    let map = src.text(MAX_MAP_BYTES)?;
    let mode = src.text(MAX_MODE_BYTES)?;
    let count = usize::from(src.get::<u8>()?);
    if count > usize::from(MAX_SESSION_MEMBERS) {
        return Err(ProtocolError::LengthExceeded(count));
    }
    let mut members = Vec::with_capacity(count);
    let mut member_names = HashMap::with_capacity(count);
    for _ in 0..count {
        let member: MemberId = src.get()?;
        member_names.insert(member, src.text(MAX_PLAYER_NAME_BYTES)?);
        members.push(member);
    }
    Ok(RoomView {
        room_id,
        name,
        host,
        members,
        member_names,
        map,
        mode,
        joinable,
        revision,
        epoch,
        phase,
        max_players,
        requires,
    })
}

fn write_advert(out: &mut Out, advert: &Advert) -> Result<(), ProtocolError> {
    out.put(advert.id).text(&advert.name, MAX_ADVERT_NAME_BYTES)?;
    write_map_mode(out, &advert.map, &advert.mode)?;
    out.put(advert.players)
        .put(advert.max_players)
        .put(advert.locked)
        .put(advert.in_match)
        .put(advert.requires)
        .put(advert.generation);
    Ok(())
}

fn read_advert(src: &mut In<'_>) -> Result<Advert, ProtocolError> {
    Ok(Advert {
        id: src.get()?,
        name: src.text(MAX_ADVERT_NAME_BYTES)?,
        map: src.text(MAX_MAP_BYTES)?,
        mode: src.text(MAX_MODE_BYTES)?,
        players: src.get()?,
        max_players: src.get()?,
        locked: src.get()?,
        in_match: src.get()?,
        requires: src.get::<ContentFlags>()?,
        generation: src.get()?,
    })
}

pub(crate) fn write_response(out: &mut Out, body: &ResponseBody) -> Result<(), ProtocolError> {
    use ResponseBody as A;
    match body {
        A::Status(status) => {
            out.put(answer::STATUS)
                .put(status.protocol_version)
                .put(status.max_opaque_payload)
                .put(status.max_session_members);
        }
        A::RoomCreated { member_id, view } => {
            write_view(out.put(answer::CREATED).put(*member_id), view)?;
        }
        A::RoomJoined { member_id, view } => {
            write_view(out.put(answer::JOINED).put(*member_id), view)?;
        }
        A::RoomLeft => {
            out.put(answer::LEFT);
        }
        A::RoomUpdated { view } => {
            write_view(out.put(answer::UPDATED), view)?;
        }
        A::RoomList {
            generation,
            adverts,
        } => {
            if adverts.len() > MAX_LIST_ADVERTS {
                return Err(ProtocolError::LengthExceeded(adverts.len()));
            }
            out.put(answer::LIST)
                .put(*generation)
                .put(adverts.len() as u16);
            for advert in adverts {
                write_advert(out, advert)?;
            }
        }
        A::Ack => {
            out.put(answer::ACK);
        }
        A::Error(refusal) => {
            out.put(answer::REFUSED).put(*refusal);
        }
    }
    Ok(())
}

pub(crate) fn read_response(src: &mut In<'_>) -> Result<ResponseBody, ProtocolError> {
    use ResponseBody as A;
    Ok(match src.get::<u8>()? {
        answer::STATUS => A::Status(crate::StatusResponse {
            protocol_version: src.get()?,
            max_opaque_payload: src.get()?,
            max_session_members: src.get()?,
        }),
        answer::CREATED => A::RoomCreated {
            member_id: src.get()?,
            view: read_view(src)?,
        },
        answer::JOINED => A::RoomJoined {
            member_id: src.get()?,
            view: read_view(src)?,
        },
        answer::LEFT => A::RoomLeft,
        answer::UPDATED => A::RoomUpdated {
            view: read_view(src)?,
        },
        answer::LIST => {
            let generation = src.get()?;
            let count = usize::from(src.get::<u16>()?);
            if count > MAX_LIST_ADVERTS {
                return Err(ProtocolError::LengthExceeded(count));
            }
            let adverts = (0..count)
                .map(|_| read_advert(src))
                .collect::<Result<Vec<_>, _>>()?;
            A::RoomList {
                generation,
                adverts,
            }
        }
        answer::ACK => A::Ack,
        answer::REFUSED => A::Error(src.get()?),
        other => return Err(ProtocolError::UnknownTag(other)),
    })
}

pub(crate) fn write_event(out: &mut Out, event: &PeerEvent) {
    use PeerEvent as E;
    match *event {
        E::HostWorldReady {
            epoch,
            map,
            weapons,
            classes,
        } => {
            out.put(relayed::WORLD_READY).put(epoch);
            write_digests(out, map, weapons, classes);
        }
        E::MapLoaded {
            member_id,
            epoch,
            map,
            weapons,
            classes,
        } => {
            out.put(relayed::MAP_LOADED).put(member_id).put(epoch);
            write_digests(out, map, weapons, classes);
        }
        E::BootstrapApplied {
            member_id,
            epoch,
            bootstrap_id,
            snapshot_seq,
            connection,
        } => {
            out.put(relayed::SNAPSHOT_APPLIED)
                .put(member_id)
                .put(epoch)
                .put(bootstrap_id)
                .put(snapshot_seq)
                .put(connection);
        }
        E::EnterMatch {
            member_id,
            epoch,
            bootstrap_id,
            connection_id,
            client_id,
        } => {
            out.put(relayed::ENTER)
                .put(member_id)
                .put(epoch)
                .put(bootstrap_id)
                .put(connection_id)
                .put(client_id);
        }
        E::VoteToSkip { member_id } => {
            out.put(relayed::SKIP_VOTE).put(member_id);
        }
        E::AdmissionFailed {
            member_id,
            epoch,
            reason,
        } => {
            out.put(relayed::ADMIT_FAILED)
                .put(member_id)
                .put(epoch)
                .put(reason);
        }
    }
}

pub(crate) fn read_event(src: &mut In<'_>) -> Result<PeerEvent, ProtocolError> {
    use PeerEvent as E;
    Ok(match src.get::<u8>()? {
        relayed::WORLD_READY => E::HostWorldReady {
            epoch: src.get()?,
            map: src.get()?,
            weapons: src.get()?,
            classes: src.get()?,
        },
        relayed::MAP_LOADED => E::MapLoaded {
            member_id: src.get()?,
            epoch: src.get()?,
            map: src.get()?,
            weapons: src.get()?,
            classes: src.get()?,
        },
        relayed::SNAPSHOT_APPLIED => E::BootstrapApplied {
            member_id: src.get()?,
            epoch: src.get()?,
            bootstrap_id: src.get()?,
            snapshot_seq: src.get()?,
            connection: src.get()?,
        },
        relayed::ENTER => E::EnterMatch {
            member_id: src.get()?,
            epoch: src.get()?,
            bootstrap_id: src.get()?,
            connection_id: src.get()?,
            client_id: src.get()?,
        },
        relayed::SKIP_VOTE => E::VoteToSkip {
            member_id: src.get()?,
        },
        relayed::ADMIT_FAILED => E::AdmissionFailed {
            member_id: src.get()?,
            epoch: src.get()?,
            reason: src.get()?,
        },
        other => return Err(ProtocolError::UnknownTag(other)),
    })
}
