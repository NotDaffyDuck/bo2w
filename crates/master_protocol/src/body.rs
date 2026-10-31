//! Body layouts, one per frame kind. Requests, responses and peer events start with a
//! one-byte tag naming the variant; the other kinds have a single layout.

use std::collections::HashMap;

use crate::bytes::{In, Out, Scalar};
use crate::{
    EntryRefusalReason, RoomListing, AssetCoverageBits, ControlGreeting, RoomOccupantId, RoomMemberNotice, ControlCodecFault,
    QueryPayload, ReplyPayload, LobbyOverview, DirectoryServiceFault, LISTING_TITLE_BYTE_CAP, VERSION_TEXT_BYTE_CAP,
    LISTING_PAGE_LIMIT, LEVEL_NAME_BYTE_CAP, RULESET_NAME_BYTE_CAP, MAX_PLAYER_NAME_BYTES, ROOM_OCCUPANT_LIMIT,
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
    pub const KICK_MEMBER: u8 = 16;
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
fn check_room(name: &str, map: &str, mode: &str, seat_capacity: u8) -> Result<(), ControlCodecFault> {
    let name_ok = (1..=LISTING_TITLE_BYTE_CAP).contains(&name.len());
    let players_ok = (2..=ROOM_OCCUPANT_LIMIT).contains(&seat_capacity);
    if name_ok && players_ok {
        check_map_mode(map, mode)
    } else {
        Err(ControlCodecFault::MalformedContent)
    }
}

fn check_map_mode(level: &str, ruleset: &str) -> Result<(), ControlCodecFault> {
    let map_ok = (1..=LEVEL_NAME_BYTE_CAP).contains(&level.len());
    let mode_ok = (1..=RULESET_NAME_BYTE_CAP).contains(&ruleset.len());
    if map_ok && mode_ok {
        Ok(())
    } else {
        Err(ControlCodecFault::MalformedContent)
    }
}

impl Scalar for EntryRefusalReason {
    fn emit(self, out: &mut Out) {
        let (code, digests) = match self {
            Self::CutoffInstant => (1u8, None),
            Self::Cancelled => (2, None),
            Self::LevelAssetHashes { host_digest: host, peer_digest: peer } => (3, Some((host, peer))),
            Self::WeaponLedger { host_digest: host, peer_digest: peer } => (4, Some((host, peer))),
            Self::LoadoutTableHashes { host_digest: host, peer_digest: peer } => (5, Some((host, peer))),
        };
        out.put(code);
        if let Some((host, peer)) = digests {
            out.put(host).put(peer);
        }
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
        use EntryRefusalReason;
        let code = src.get::<u8>()?;
        let pair = |src: &mut In<'_>| -> Result<(u64, u64), ControlCodecFault> {
            Ok((src.get()?, src.get()?))
        };
        Ok(match code {
            1 => EntryRefusalReason::CutoffInstant,
            2 => EntryRefusalReason::Cancelled,
            3 => {
                let (host, peer) = pair(src)?;
                EntryRefusalReason::LevelAssetHashes { host_digest: host, peer_digest: peer }
            }
            4 => {
                let (host, peer) = pair(src)?;
                EntryRefusalReason::WeaponLedger { host_digest: host, peer_digest: peer }
            }
            5 => {
                let (host, peer) = pair(src)?;
                EntryRefusalReason::LoadoutTableHashes { host_digest: host, peer_digest: peer }
            }
            _ => return Err(ControlCodecFault::MalformedContent),
        })
    }
}

impl Scalar for DirectoryServiceFault {
    fn emit(self, out: &mut Out) {
        use DirectoryServiceFault;
        let code: u8 = match self {
            DirectoryServiceFault::GarbledRequest => 1,
            DirectoryServiceFault::ListingRejected => 2,
            DirectoryServiceFault::ListingsFull => 3,
            DirectoryServiceFault::HostSlotTaken => 4,
            DirectoryServiceFault::NoSuchListing => 5,
            DirectoryServiceFault::Full => 7,
            DirectoryServiceFault::Locked => 8,
            DirectoryServiceFault::NotRoomOwner => 11,
            DirectoryServiceFault::NotRoomOccupant => 13,
            DirectoryServiceFault::AbsentResource { requires, have: held } => {
                out.put(14u8).put(requires).put(held);
                return;
            }
        };
        out.put(code);
    }

    fn parse(src: &mut In<'_>) -> Result<Self, ControlCodecFault> {
        use DirectoryServiceFault;
        Ok(match src.get::<u8>()? {
            1 => DirectoryServiceFault::GarbledRequest,
            2 => DirectoryServiceFault::ListingRejected,
            3 => DirectoryServiceFault::ListingsFull,
            4 => DirectoryServiceFault::HostSlotTaken,
            5 => DirectoryServiceFault::NoSuchListing,
            7 => DirectoryServiceFault::Full,
            8 => DirectoryServiceFault::Locked,
            11 => DirectoryServiceFault::NotRoomOwner,
            13 => DirectoryServiceFault::NotRoomOccupant,
            14 => DirectoryServiceFault::AbsentResource {
                requires: src.get()?,
                have: src.get()?,
            },
            _ => return Err(ControlCodecFault::MalformedContent),
        })
    }
}

pub(crate) fn write_hello(out: &mut Out, hello: &ControlGreeting) -> Result<(), ControlCodecFault> {
    out.put(hello.wire_rev)
        .put(hello.match_wire_version)
        .put(hello.role)
        .put_text(&hello.build, VERSION_TEXT_BYTE_CAP)?
        .put_text(&hello.display_name, MAX_PLAYER_NAME_BYTES)?;
    Ok(())
}

pub(crate) fn read_hello(src: &mut In<'_>) -> Result<ControlGreeting, ControlCodecFault> {
    Ok(ControlGreeting {
        wire_rev: src.get()?,
        match_wire_version: src.get()?,
        role: src.get()?,
        build: src.text(VERSION_TEXT_BYTE_CAP)?,
        display_name: src.text(MAX_PLAYER_NAME_BYTES)?,
    })
}

/// Map, mode: the pair shared by create, options and start.
fn write_map_mode(out: &mut Out, level: &str, ruleset: &str) -> Result<(), ControlCodecFault> {
    out.put_text(level, LEVEL_NAME_BYTE_CAP)?.put_text(ruleset, RULESET_NAME_BYTE_CAP)?;
    Ok(())
}

/// Map, weapon-table and class-catalog digests, in that order.
fn write_digests(out: &mut Out, level: u64, weapon_table: u64, class_table: u64) {
    out.put(level).put(weapon_table).put(class_table);
}

pub(crate) fn write_request(out: &mut Out, body: &QueryPayload) -> Result<(), ControlCodecFault> {
    match body {
        QueryPayload::Status => {
            out.put(ask::STATUS);
        }
        QueryPayload::OpenLobby {
            name,
            map,
            mode,
            seat_capacity,
            requires,
        } => {
            check_room(name, map, mode, *seat_capacity)?;
            out.put(ask::CREATE).put_text(name, LISTING_TITLE_BYTE_CAP)?;
            write_map_mode(out, map, mode)?;
            out.put(*seat_capacity).put(*requires);
        }
        QueryPayload::JoinRoom { lobby_number, have } => {
            out.put(ask::JOIN).put(*lobby_number).put(*have);
        }
        QueryPayload::LeaveRoom => {
            out.put(ask::LEAVE);
        }
        QueryPayload::ConfigureLobby {
            map,
            mode,
            open_to_join,
        } => {
            check_map_mode(map, mode)?;
            write_map_mode(out.put(ask::OPTIONS), map, mode)?;
            out.put(*open_to_join);
        }
        QueryPayload::BeginGame { map, mode } => {
            check_map_mode(map, mode)?;
            write_map_mode(out.put(ask::START), map, mode)?;
        }
        QueryPayload::EndMatch { lobby_number, epoch } => {
            out.put(ask::END).put(*lobby_number).put(*epoch);
        }
        QueryPayload::CloseRoom => {
            out.put(ask::CLOSE);
        }
        QueryPayload::ListRooms => {
            out.put(ask::LIST);
        }
        QueryPayload::HostMapLoaded {
            lobby_number,
            epoch,
            map,
            weapons,
            classes,
        } => {
            out.put(ask::WORLD_READY).put(*lobby_number).put(*epoch);
            write_digests(out, *map, *weapons, *classes);
        }
        QueryPayload::MapLoaded {
            epoch,
            map,
            weapons,
            classes,
        } => {
            out.put(ask::MAP_LOADED).put(*epoch);
            write_digests(out, *map, *weapons, *classes);
        }
        QueryPayload::TransferInstalledMsg {
            epoch,
            bootstrap_id: boot,
            frame_number_on_wire,
            connection,
        } => {
            out.put(ask::SNAPSHOT_APPLIED)
                .put(*epoch)
                .put(*boot)
                .put(*frame_number_on_wire)
                .put(*connection);
        }
        QueryPayload::JoinGame {
            member_id,
            epoch,
            bootstrap_id: boot,
            connection_id: conn,
            player_slot_no,
        } => {
            out.put(ask::ENTER)
                .put(*member_id)
                .put(*epoch)
                .put(*boot)
                .put(*conn)
                .put(*player_slot_no);
        }
        QueryPayload::BallotForSkip => {
            out.put(ask::SKIP_VOTE);
        }
        QueryPayload::KickMember { member_id } => {
            out.put(ask::KICK_MEMBER).put(*member_id);
        }
        QueryPayload::EntryRefused {
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

pub(crate) fn read_request(src: &mut In<'_>) -> Result<QueryPayload, ControlCodecFault> {
    Ok(match src.get::<u8>()? {
        ask::STATUS => QueryPayload::Status,
        ask::CREATE => QueryPayload::OpenLobby {
            name: src.text(LISTING_TITLE_BYTE_CAP)?,
            map: src.text(LEVEL_NAME_BYTE_CAP)?,
            mode: src.text(RULESET_NAME_BYTE_CAP)?,
            seat_capacity: src.get()?,
            requires: src.get()?,
        },
        ask::JOIN => QueryPayload::JoinRoom {
            lobby_number: src.get()?,
            have: src.get()?,
        },
        ask::LEAVE => QueryPayload::LeaveRoom,
        ask::OPTIONS => QueryPayload::ConfigureLobby {
            map: src.text(LEVEL_NAME_BYTE_CAP)?,
            mode: src.text(RULESET_NAME_BYTE_CAP)?,
            open_to_join: src.get()?,
        },
        ask::START => QueryPayload::BeginGame {
            map: src.text(LEVEL_NAME_BYTE_CAP)?,
            mode: src.text(RULESET_NAME_BYTE_CAP)?,
        },
        ask::END => QueryPayload::EndMatch {
            lobby_number: src.get()?,
            epoch: src.get()?,
        },
        ask::CLOSE => QueryPayload::CloseRoom,
        ask::LIST => QueryPayload::ListRooms,
        ask::WORLD_READY => QueryPayload::HostMapLoaded {
            lobby_number: src.get()?,
            epoch: src.get()?,
            map: src.get()?,
            weapons: src.get()?,
            classes: src.get()?,
        },
        ask::MAP_LOADED => QueryPayload::MapLoaded {
            epoch: src.get()?,
            map: src.get()?,
            weapons: src.get()?,
            classes: src.get()?,
        },
        ask::SNAPSHOT_APPLIED => QueryPayload::TransferInstalledMsg {
            epoch: src.get()?,
            bootstrap_id: src.get()?,
            frame_number_on_wire: src.get()?,
            connection: src.get()?,
        },
        ask::ENTER => QueryPayload::JoinGame {
            member_id: src.get()?,
            epoch: src.get()?,
            bootstrap_id: src.get()?,
            connection_id: src.get()?,
            player_slot_no: src.get()?,
        },
        ask::SKIP_VOTE => QueryPayload::BallotForSkip,
        ask::KICK_MEMBER => QueryPayload::KickMember {
            member_id: src.get()?,
        },
        ask::ADMIT_FAILED => QueryPayload::EntryRefused {
            member_id: src.get()?,
            epoch: src.get()?,
            reason: src.get()?,
        },
        other => return Err(ControlCodecFault::UnrecognizedKindByte(other)),
    })
}

pub(crate) fn write_view(out: &mut Out, view: &LobbyOverview) -> Result<(), ControlCodecFault> {
    let count = u8::try_from(view.members.len())
        .ok()
        .filter(|&count| count <= ROOM_OCCUPANT_LIMIT)
        .ok_or(ControlCodecFault::SizeOverLimit(view.members.len()))?;
    let LobbyOverview { name, map, mode, seat_capacity, .. } = view;
    check_room(name, map, mode, *seat_capacity)?;
    out.put(view.lobby_number)
        .put(view.host)
        .put(view.revision)
        .put(view.epoch)
        .put(view.phase)
        .put(view.open_to_join)
        .put(view.seat_capacity)
        .put(view.requires)
        .put_text(name, LISTING_TITLE_BYTE_CAP)?;
    write_map_mode(out, map, mode)?;
    out.put(count);
    for &member in &view.members {
        let shown = view
            .member_names
            .get(&member)
            .map_or(UNNAMED_MEMBER, String::as_str);
        out.put(member).put_text(shown, MAX_PLAYER_NAME_BYTES)?;
    }
    Ok(())
}

pub(crate) fn read_view(src: &mut In<'_>) -> Result<LobbyOverview, ControlCodecFault> {
    let lobby_number = src.get()?;
    let host = src.get()?;
    let revision = src.get()?;
    let epoch = src.get()?;
    let phase = src.get()?;
    let open_to_join = src.get()?;
    let seat_capacity = src.get()?;
    let requires = src.get()?;
    let name = src.text(LISTING_TITLE_BYTE_CAP)?;
    let map = src.text(LEVEL_NAME_BYTE_CAP)?;
    let mode = src.text(RULESET_NAME_BYTE_CAP)?;
    let count = usize::from(src.get::<u8>()?);
    if count > usize::from(ROOM_OCCUPANT_LIMIT) {
        return Err(ControlCodecFault::SizeOverLimit(count));
    }
    let mut members = Vec::with_capacity(count);
    let mut member_names = HashMap::with_capacity(count);
    for _ in 0..count {
        let member: RoomOccupantId = src.get()?;
        member_names.insert(member, src.text(MAX_PLAYER_NAME_BYTES)?);
        members.push(member);
    }
    Ok(LobbyOverview {
        lobby_number,
        name,
        host,
        members,
        member_names,
        map,
        mode,
        open_to_join,
        revision,
        epoch,
        phase,
        seat_capacity,
        requires,
    })
}

fn write_advert(out: &mut Out, advert: &RoomListing) -> Result<(), ControlCodecFault> {
    out.put(advert.id).put_text(&advert.name, LISTING_TITLE_BYTE_CAP)?;
    write_map_mode(out, &advert.map, &advert.mode)?;
    out.put(advert.players)
        .put(advert.seat_capacity)
        .put(advert.locked)
        .put(advert.playing)
        .put(advert.requires)
        .put(advert.generation);
    Ok(())
}

fn read_advert(src: &mut In<'_>) -> Result<RoomListing, ControlCodecFault> {
    Ok(RoomListing {
        id: src.get()?,
        name: src.text(LISTING_TITLE_BYTE_CAP)?,
        map: src.text(LEVEL_NAME_BYTE_CAP)?,
        mode: src.text(RULESET_NAME_BYTE_CAP)?,
        players: src.get()?,
        seat_capacity: src.get()?,
        locked: src.get()?,
        playing: src.get()?,
        requires: src.get::<AssetCoverageBits>()?,
        generation: src.get()?,
    })
}

pub(crate) fn write_response(out: &mut Out, body: &ReplyPayload) -> Result<(), ControlCodecFault> {
    match body {
        ReplyPayload::Status(status) => {
            out.put(answer::STATUS)
                .put(status.wire_rev)
                .put(status.opaque_body_ceiling)
                .put(status.member_count_ceiling);
        }
        ReplyPayload::LobbyOpened { member_id: seat, view } => {
            write_view(out.put(answer::CREATED).put(*seat), view)?;
        }
        ReplyPayload::LobbyEntered { member_id: seat, view } => {
            write_view(out.put(answer::JOINED).put(*seat), view)?;
        }
        ReplyPayload::RoomLeft => {
            out.put(answer::LEFT);
        }
        ReplyPayload::RoomUpdated { view } => {
            write_view(out.put(answer::UPDATED), view)?;
        }
        ReplyPayload::RoomList {
            generation: stamp,
            adverts,
        } => {
            if adverts.len() > LISTING_PAGE_LIMIT {
                return Err(ControlCodecFault::SizeOverLimit(adverts.len()));
            }
            out.put(answer::LIST)
                .put(*stamp)
                .put(adverts.len() as u16);
            for listing in adverts {
                write_advert(out, listing)?;
            }
        }
        ReplyPayload::Ack => {
            out.put(answer::ACK);
        }
        ReplyPayload::Error(refusal) => {
            out.put(answer::REFUSED).put(*refusal);
        }
    }
    Ok(())
}

pub(crate) fn read_response(src: &mut In<'_>) -> Result<ReplyPayload, ControlCodecFault> {
    Ok(match src.get::<u8>()? {
        answer::STATUS => ReplyPayload::Status(crate::HealthReply {
            wire_rev: src.get()?,
            opaque_body_ceiling: src.get()?,
            member_count_ceiling: src.get()?,
        }),
        answer::CREATED => ReplyPayload::LobbyOpened {
            member_id: src.get()?,
            view: read_view(src)?,
        },
        answer::JOINED => ReplyPayload::LobbyEntered {
            member_id: src.get()?,
            view: read_view(src)?,
        },
        answer::LEFT => ReplyPayload::RoomLeft,
        answer::UPDATED => ReplyPayload::RoomUpdated {
            view: read_view(src)?,
        },
        answer::LIST => {
            let stamp = src.get()?;
            let count = usize::from(src.get::<u16>()?);
            if count > LISTING_PAGE_LIMIT {
                return Err(ControlCodecFault::SizeOverLimit(count));
            }
            let adverts = (0..count)
                .map(|_| read_advert(src))
                .collect::<Result<Vec<_>, _>>()?;
            ReplyPayload::RoomList {
                generation: stamp,
                adverts,
            }
        }
        answer::ACK => ReplyPayload::Ack,
        answer::REFUSED => ReplyPayload::Error(src.get()?),
        other => return Err(ControlCodecFault::UnrecognizedKindByte(other)),
    })
}

pub(crate) fn write_event(out: &mut Out, event: &RoomMemberNotice) {
    match *event {
        RoomMemberNotice::HostMapLoaded {
            epoch,
            map,
            weapon_digest: weapons,
            class_digest: classes,
        } => {
            out.put(relayed::WORLD_READY).put(epoch);
            write_digests(out, map, weapons, classes);
        }
        RoomMemberNotice::MapLoaded {
            member_id,
            epoch,
            map,
            weapon_digest: weapons,
            class_digest: classes,
        } => {
            out.put(relayed::MAP_LOADED).put(member_id).put(epoch);
            write_digests(out, map, weapons, classes);
        }
        RoomMemberNotice::TransferInstalledMsg {
            member_id,
            epoch,
            transfer_id: boot,
            frame_number_on_wire,
            connection,
        } => {
            out.put(relayed::SNAPSHOT_APPLIED)
                .put(member_id)
                .put(epoch)
                .put(boot)
                .put(frame_number_on_wire)
                .put(connection);
        }
        RoomMemberNotice::JoinGame {
            member_id,
            epoch,
            transfer_id: boot,
            link_id: conn,
            player_slot_no,
        } => {
            out.put(relayed::ENTER)
                .put(member_id)
                .put(epoch)
                .put(boot)
                .put(conn)
                .put(player_slot_no);
        }
        RoomMemberNotice::BallotForSkip { member_id } => {
            out.put(relayed::SKIP_VOTE).put(member_id);
        }
        RoomMemberNotice::EntryRefused {
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

pub(crate) fn read_event(src: &mut In<'_>) -> Result<RoomMemberNotice, ControlCodecFault> {
    Ok(match src.get::<u8>()? {
        relayed::WORLD_READY => RoomMemberNotice::HostMapLoaded {
            epoch: src.get()?,
            map: src.get()?,
            weapon_digest: src.get()?,
            class_digest: src.get()?,
        },
        relayed::MAP_LOADED => RoomMemberNotice::MapLoaded {
            member_id: src.get()?,
            epoch: src.get()?,
            map: src.get()?,
            weapon_digest: src.get()?,
            class_digest: src.get()?,
        },
        relayed::SNAPSHOT_APPLIED => RoomMemberNotice::TransferInstalledMsg {
            member_id: src.get()?,
            epoch: src.get()?,
            transfer_id: src.get()?,
            frame_number_on_wire: src.get()?,
            connection: src.get()?,
        },
        relayed::ENTER => RoomMemberNotice::JoinGame {
            member_id: src.get()?,
            epoch: src.get()?,
            transfer_id: src.get()?,
            link_id: src.get()?,
            player_slot_no: src.get()?,
        },
        relayed::SKIP_VOTE => RoomMemberNotice::BallotForSkip {
            member_id: src.get()?,
        },
        relayed::ADMIT_FAILED => RoomMemberNotice::EntryRefused {
            member_id: src.get()?,
            epoch: src.get()?,
            reason: src.get()?,
        },
        other => return Err(ControlCodecFault::UnrecognizedKindByte(other)),
    })
}
