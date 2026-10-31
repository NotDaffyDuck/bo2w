//! Byte-exact checks of the lobby wire format. Every expected byte string is
//! assembled by hand here, so a codec change that moves a single byte fails.

use std::collections::HashMap;

use master_protocol::*;

const RID: u64 = 0x1122_3344_5566_7788;

fn id(fill: u8) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = fill.wrapping_add(i as u8);
    }
    out
}

/// Tiny independent byte builder for the expected side.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn b(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn w(mut self, v: u16) -> Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn d(mut self, v: u32) -> Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn q(mut self, v: u64) -> Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn raw(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self
    }
    fn s(self, v: &str) -> Self {
        self.b(v.len() as u8).raw(v.as_bytes())
    }
}

/// The request id the encoder tests use.
const ASK_ID: u64 = 1;

fn framed(kind: u8, rid: u64, body: &[u8]) -> Vec<u8> {
    Bytes::default()
        .raw(b"BO2M")
        .w(10)
        .b(kind)
        .q(rid)
        .w(body.len() as u16)
        .raw(body)
        .0
}

fn round_trip(frame: ControlEnvelope, expected: Vec<u8>) {
    let bytes = encode_control_frame(&frame).expect("encode");
    assert_eq!(bytes, expected, "encoded bytes for {frame:?}");
    assert_eq!(decode_control_frame(&bytes).expect("decode"), frame);
    let streamed = write_stream_frame(&frame).expect("stream encode");
    assert_eq!(&streamed[..4], &(expected.len() as u32).to_be_bytes());
    assert_eq!(&streamed[4..], &expected[..]);
    assert_eq!(
        stream_frame_len(streamed[..4].try_into().unwrap()),
        Ok(expected.len())
    );
    assert_eq!(decode_control_frame(&streamed[4..]).expect("payload"), frame);
}

fn request(body: QueryPayload, expected_body: Vec<u8>) {
    round_trip(
        ControlEnvelope::Request(ControlQuery {
            call_id: RID,
            body,
        }),
        framed(1, RID, &expected_body),
    );
}

fn response(body: ReplyPayload, expected_body: Vec<u8>) {
    round_trip(
        ControlEnvelope::Response(ControlReply {
            call_id: RID,
            body,
        }),
        framed(2, RID, &expected_body),
    );
}

fn event(ev: RoomMemberNotice, expected_body: Vec<u8>) {
    round_trip(ControlEnvelope::RoomMemberNotice(ev), framed(5, 0, &expected_body));
}

fn sample_view() -> LobbyOverview {
    let a = RoomOccupantId(id(0x40));
    let b = RoomOccupantId(id(0x60));
    let mut names = HashMap::new();
    names.insert(a, "Ann".to_owned());
    names.insert(b, "Bo".to_owned());
    LobbyOverview {
        lobby_number: ListingId(id(0x10)),
        name: "Room".to_owned(),
        host: a,
        members: vec![a, b],
        member_names: names,
        map: "mp_raid".to_owned(),
        mode: "tdm".to_owned(),
        open_to_join: true,
        revision: 0x0102_0304_0506_0708,
        epoch: 7,
        phase: LobbyStage::Loading,
        seat_capacity: 12,
        requires: AssetCoverageBits(0b1000),
    }
}

fn sample_view_bytes() -> Bytes {
    Bytes::default()
        .raw(&id(0x10))
        .raw(&id(0x40))
        .q(0x0102_0304_0506_0708)
        .d(7)
        .b(1)
        .b(1)
        .b(12)
        .b(0b1000)
        .s("Room")
        .s("mp_raid")
        .s("tdm")
        .b(2)
        .raw(&id(0x40))
        .s("Ann")
        .raw(&id(0x60))
        .s("Bo")
}

#[test]
fn limits_and_versions() {
    assert_eq!(WIRE_REV, 10);
    assert_eq!(ALPN, b"bo2mp-master/10");
    assert_eq!(CONTROL_MESSAGE_SIZE_LIMIT, 16384);
    assert_eq!(OPAQUE_BODY_BYTE_CAP, 1100);
    assert_eq!(TRANSFER_SIZE_LIMIT, 262_144);
    assert_eq!(TRANSFER_STREAM_CAP, 262_162);
    assert_eq!(FORWARD_ONEWAY_CHANNEL_CAP, 8);
    assert_eq!(PARALLEL_TRANSFER_LIMIT, 4);
    assert_eq!(ROOM_OCCUPANT_LIMIT, 18);
    assert_eq!(LISTING_TITLE_BYTE_CAP, 48);
    assert_eq!(LEVEL_NAME_BYTE_CAP, 64);
    assert_eq!(RULESET_NAME_BYTE_CAP, 48);
    assert_eq!(VERSION_TEXT_BYTE_CAP, 64);
    assert_eq!(MAX_PLAYER_NAME_BYTES, 64);
    assert_eq!(LISTING_PAGE_LIMIT, 128);
    assert_eq!(LINK_IDLE_LIMIT, std::time::Duration::from_secs(8));
    assert_eq!(LINK_PING_EVERY, std::time::Duration::from_secs(1));
    assert_eq!(
        HealthReply::CURRENT,
        HealthReply {
            wire_rev: 10,
            opaque_body_ceiling: 1100,
            member_count_ceiling: 18,
        }
    );
}

#[test]
fn channels() {
    assert_eq!(Channel::ALL, [Channel::Prod, Channel::PreReleaseBuild]);
    let rows = [
        (Channel::Prod, "prod", 4433, "bo2mp-master.service", "bo2mp-prod"),
        (Channel::PreReleaseBuild, "dev", 4434, "bo2mp-master-dev.service", "bo2mp-dev"),
    ];
    for (ch, name, port, unit, san) in rows {
        assert_eq!(ch.as_str(), name);
        assert_eq!(ch.to_string(), name);
        assert_eq!(ch.port(), port);
        assert_eq!(ch.systemd_unit(), unit);
        assert_eq!(ch.tls_name(), san);
        assert_eq!(name.parse::<Channel>(), Ok(ch));
    }
    let err = "staging".parse::<Channel>().unwrap_err();
    assert_eq!(err.to_string(), "channel must be either prod or dev");
    assert!("Prod".parse::<Channel>().is_err());
    let _: &dyn std::error::Error = &err;
}

#[test]
fn ids_print_and_parse_as_hex() {
    let a = ListingId(id(0xa0));
    let text = a.to_string();
    assert_eq!(text, "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf");
    assert_eq!(text.parse::<ListingId>(), Ok(a));
    assert_eq!(text.to_uppercase().parse::<ListingId>(), Ok(a));
    let m = RoomOccupantId([0; 16]);
    assert_eq!(m.to_string(), "0".repeat(32));
    assert_eq!("0".repeat(32).parse::<RoomOccupantId>(), Ok(m));
    assert_eq!(ListingId::default(), ListingId([0; 16]));
    for bad in ["", "00", &"0".repeat(31), &"0".repeat(33), &format!("{}zz", "0".repeat(30))] {
        assert_eq!(bad.parse::<ListingId>(), Err(ControlCodecFault::InvalidId), "{bad:?}");
        assert_eq!(bad.parse::<RoomOccupantId>(), Err(ControlCodecFault::InvalidId), "{bad:?}");
    }
    assert!(ListingId(id(1)) < ListingId(id(2)));
}

#[test]
fn pack_presence_bits() {
    let need = AssetCoverageBits(0b1100);
    assert_eq!(need.lacking_in(AssetCoverageBits(0b0100)), AssetCoverageBits(0b1000));
    assert!(need.lacking_in(AssetCoverageBits(0xff)).is_empty());
    assert!(AssetCoverageBits::default().is_empty());
    assert!(!need.is_empty());
}

#[test]
fn hello_bytes() {
    let hello = ControlGreeting {
        wire_rev: 10,
        match_wire_version: 0x0102_0304,
        role: PeerKindTag::Host,
        build: "b1".to_owned(),
        display_name: "Al".to_owned(),
    };
    let body = Bytes::default().w(10).d(0x0102_0304).b(1).s("b1").s("Al").0;
    round_trip(ControlEnvelope::Hello(hello.clone()), framed(0, 0, &body));
    for (role, wire) in [(PeerKindTag::Cli, 0), (PeerKindTag::Host, 1), (PeerKindTag::Join, 2)] {
        let frame = ControlEnvelope::Hello(ControlGreeting { role, ..hello.clone() });
        let body = Bytes::default().w(10).d(0x0102_0304).b(wire).s("b1").s("Al").0;
        round_trip(frame, framed(0, 0, &body));
    }
}

#[test]
fn request_bytes() {
    request(QueryPayload::Status, vec![1]);
    request(
        QueryPayload::OpenLobby {
            name: "Room".to_owned(),
            map: "mp_raid".to_owned(),
            mode: "tdm".to_owned(),
            seat_capacity: 18,
            requires: AssetCoverageBits(8),
        },
        Bytes::default().b(2).s("Room").s("mp_raid").s("tdm").b(18).b(8).0,
    );
    request(
        QueryPayload::JoinRoom {
            lobby_number: ListingId(id(0x10)),
            have: AssetCoverageBits(12),
        },
        Bytes::default().b(3).raw(&id(0x10)).b(12).0,
    );
    request(QueryPayload::LeaveRoom, vec![4]);
    request(
        QueryPayload::ConfigureLobby {
            map: "mp_slums".to_owned(),
            mode: "dom".to_owned(),
            open_to_join: false,
        },
        Bytes::default().b(5).s("mp_slums").s("dom").b(0).0,
    );
    request(
        QueryPayload::ConfigureLobby {
            map: "m".to_owned(),
            mode: "d".to_owned(),
            open_to_join: true,
        },
        Bytes::default().b(5).s("m").s("d").b(1).0,
    );
    request(
        QueryPayload::BeginGame {
            map: "mp_raid".to_owned(),
            mode: "tdm".to_owned(),
        },
        Bytes::default().b(6).s("mp_raid").s("tdm").0,
    );
    request(
        QueryPayload::EndMatch {
            lobby_number: ListingId(id(0x20)),
            epoch: 0xdead_beef,
        },
        Bytes::default().b(7).raw(&id(0x20)).d(0xdead_beef).0,
    );
    request(QueryPayload::CloseRoom, vec![8]);
    request(QueryPayload::ListRooms, vec![9]);
    request(
        QueryPayload::HostMapLoaded {
            lobby_number: ListingId(id(0x30)),
            epoch: 3,
            map: 0xa,
            weapons: 0xb,
            classes: 0xc,
        },
        Bytes::default().b(10).raw(&id(0x30)).d(3).q(0xa).q(0xb).q(0xc).0,
    );
    request(
        QueryPayload::MapLoaded {
            epoch: 4,
            map: 1,
            weapons: 2,
            classes: 3,
        },
        Bytes::default().b(11).d(4).q(1).q(2).q(3).0,
    );
    request(
        QueryPayload::TransferInstalledMsg {
            epoch: 5,
            bootstrap_id: 6,
            frame_number_on_wire: 7,
            connection: 8,
        },
        Bytes::default().b(12).d(5).d(6).d(7).q(8).0,
    );
    request(
        QueryPayload::JoinGame {
            member_id: RoomOccupantId(id(0x50)),
            epoch: 1,
            bootstrap_id: 2,
            connection_id: 3,
            player_slot_no: 4,
        },
        Bytes::default().b(13).raw(&id(0x50)).d(1).d(2).q(3).d(4).0,
    );
    request(QueryPayload::BallotForSkip, vec![14]);
    request(
        QueryPayload::KickMember {
            member_id: RoomOccupantId(id(0x50)),
        },
        Bytes::default().b(16).raw(&id(0x50)).0,
    );
    let reasons = [
        (EntryRefusalReason::CutoffInstant, Bytes::default().b(1)),
        (EntryRefusalReason::Cancelled, Bytes::default().b(2)),
        (
            EntryRefusalReason::LevelAssetHashes { host_digest: 1, peer_digest: 2 },
            Bytes::default().b(3).q(1).q(2),
        ),
        (
            EntryRefusalReason::WeaponLedger { host_digest: 3, peer_digest: 4 },
            Bytes::default().b(4).q(3).q(4),
        ),
        (
            EntryRefusalReason::LoadoutTableHashes { host_digest: 5, peer_digest: 6 },
            Bytes::default().b(5).q(5).q(6),
        ),
    ];
    for (reason, tail) in reasons {
        request(
            QueryPayload::EntryRefused {
                member_id: RoomOccupantId(id(0x70)),
                epoch: 9,
                reason,
            },
            Bytes::default().b(15).raw(&id(0x70)).d(9).raw(&tail.0).0,
        );
    }
}

#[test]
fn response_bytes() {
    response(
        ReplyPayload::Status(HealthReply::CURRENT),
        Bytes::default().b(1).w(10).w(1100).b(18).0,
    );
    response(
        ReplyPayload::LobbyOpened {
            member_id: RoomOccupantId(id(0x40)),
            view: sample_view(),
        },
        Bytes::default().b(2).raw(&id(0x40)).raw(&sample_view_bytes().0).0,
    );
    response(
        ReplyPayload::LobbyEntered {
            member_id: RoomOccupantId(id(0x60)),
            view: sample_view(),
        },
        Bytes::default().b(3).raw(&id(0x60)).raw(&sample_view_bytes().0).0,
    );
    response(ReplyPayload::RoomLeft, vec![4]);
    response(
        ReplyPayload::RoomUpdated { view: sample_view() },
        Bytes::default().b(5).raw(&sample_view_bytes().0).0,
    );
    let playing = false;
    let advert = RoomListing {
        id: ListingId(id(0x10)),
        name: "Room".to_owned(),
        map: "mp_raid".to_owned(),
        mode: "tdm".to_owned(),
        players: 3,
        seat_capacity: 12,
        locked: true,
        playing,
        requires: AssetCoverageBits(8),
        generation: 77,
    };
    let mut started = advert.clone();
    started.locked = false;
    started.playing = true;
    response(
        ReplyPayload::RoomList {
            generation: 0x99,
            adverts: vec![advert, started],
        },
        Bytes::default()
            .b(9)
            .q(0x99)
            .w(2)
            .raw(&id(0x10))
            .s("Room")
            .s("mp_raid")
            .s("tdm")
            .b(3)
            .b(12)
            .b(1)
            .b(0)
            .b(8)
            .q(77)
            .raw(&id(0x10))
            .s("Room")
            .s("mp_raid")
            .s("tdm")
            .b(3)
            .b(12)
            .b(0)
            .b(1)
            .b(8)
            .q(77)
            .0,
    );
    let empty_page = 0;
    response(
        ReplyPayload::RoomList {
            generation: empty_page,
            adverts: Vec::new(),
        },
        Bytes::default().b(9).q(0).w(0).0,
    );
    response(ReplyPayload::Ack, vec![10]);
    let errors = [
        (DirectoryServiceFault::GarbledRequest, vec![1]),
        (DirectoryServiceFault::ListingRejected, vec![2]),
        (DirectoryServiceFault::ListingsFull, vec![3]),
        (DirectoryServiceFault::HostSlotTaken, vec![4]),
        (DirectoryServiceFault::NoSuchListing, vec![5]),
        (DirectoryServiceFault::Full, vec![7]),
        (DirectoryServiceFault::Locked, vec![8]),
        (DirectoryServiceFault::NotRoomOwner, vec![11]),
        (DirectoryServiceFault::NotRoomOccupant, vec![13]),
        (
            DirectoryServiceFault::AbsentResource {
                requires: AssetCoverageBits(12),
                have: AssetCoverageBits(4),
            },
            vec![14, 12, 4],
        ),
    ];
    for (err, tail) in errors {
        response(ReplyPayload::Error(err), Bytes::default().b(0x7f).raw(&tail).0);
    }
}

#[test]
fn room_view_frame_and_default_names() {
    round_trip(ControlEnvelope::LobbyOverview(sample_view()), framed(3, 0, &sample_view_bytes().0));
    // A member without a stored name goes out as "Player" and comes back named.
    let mut view = sample_view();
    view.member_names.clear();
    let bytes = encode_control_frame(&ControlEnvelope::LobbyOverview(view.clone())).unwrap();
    let ControlEnvelope::LobbyOverview(back) = decode_control_frame(&bytes).unwrap() else {
        panic!("not a room view");
    };
    assert_eq!(back.member_names.len(), 2);
    assert!(back.member_names.values().all(|n| n == "Player"));
    assert_eq!(back.members, view.members);
    for (phase, wire) in [(LobbyStage::Gathering, 0u8), (LobbyStage::Loading, 1), (LobbyStage::Running, 2)] {
        let v = LobbyOverview { phase, ..sample_view() };
        let enc = encode_control_frame(&ControlEnvelope::LobbyOverview(v.clone())).unwrap();
        assert_eq!(enc[17 + 16 + 16 + 8 + 4], wire);
        assert_eq!(decode_control_frame(&enc).unwrap(), ControlEnvelope::LobbyOverview(v));
    }
}

#[test]
fn closed_bytes() {
    for (reason, wire) in [
        (RoomShutdownCause::HostLeft, 0u8),
        (RoomShutdownCause::TenancyLapsed, 1),
        (RoomShutdownCause::Overload, 2),
        (RoomShutdownCause::Kicked, 3),
    ] {
        round_trip(
            ControlEnvelope::Closed {
                lobby_number: ListingId(id(0x10)),
                epoch: 0x0a0b_0c0d,
                reason,
            },
            framed(4, 0, &Bytes::default().raw(&id(0x10)).d(0x0a0b_0c0d).b(wire).0),
        );
    }
}

#[test]
fn peer_event_bytes() {
    event(
        RoomMemberNotice::HostMapLoaded {
            epoch: 1,
            map: 2,
            weapon_digest: 3,
            class_digest: 4,
        },
        Bytes::default().b(1).d(1).q(2).q(3).q(4).0,
    );
    event(
        RoomMemberNotice::MapLoaded {
            member_id: RoomOccupantId(id(0x50)),
            epoch: 1,
            map: 2,
            weapon_digest: 3,
            class_digest: 4,
        },
        Bytes::default().b(2).raw(&id(0x50)).d(1).q(2).q(3).q(4).0,
    );
    event(
        RoomMemberNotice::TransferInstalledMsg {
            member_id: RoomOccupantId(id(0x50)),
            epoch: 1,
            transfer_id: 2,
            frame_number_on_wire: 3,
            connection: 4,
        },
        Bytes::default().b(3).raw(&id(0x50)).d(1).d(2).d(3).q(4).0,
    );
    event(
        RoomMemberNotice::JoinGame {
            member_id: RoomOccupantId(id(0x50)),
            epoch: 1,
            transfer_id: 2,
            link_id: 3,
            player_slot_no: 4,
        },
        Bytes::default().b(4).raw(&id(0x50)).d(1).d(2).q(3).d(4).0,
    );
    event(
        RoomMemberNotice::BallotForSkip {
            member_id: RoomOccupantId(id(0x50)),
        },
        Bytes::default().b(5).raw(&id(0x50)).0,
    );
    event(
        RoomMemberNotice::EntryRefused {
            member_id: RoomOccupantId(id(0x50)),
            epoch: 6,
            reason: EntryRefusalReason::LoadoutTableHashes { host_digest: 7, peer_digest: 8 },
        },
        Bytes::default().b(6).raw(&id(0x50)).d(6).b(5).q(7).q(8).0,
    );
}

#[test]
fn relay_frame_inside_control_stream() {
    let payload = vec![0xee; 40];
    round_trip(ControlEnvelope::PassAlong(payload.clone()), framed(6, 0, &payload));
    round_trip(ControlEnvelope::PassAlong(Vec::new()), framed(6, 0, &[]));
    let largest = vec![1u8; CONTROL_MESSAGE_SIZE_LIMIT - 17];
    assert!(encode_control_frame(&ControlEnvelope::PassAlong(largest)).is_ok());
    let over = vec![1u8; CONTROL_MESSAGE_SIZE_LIMIT - 16];
    assert_eq!(
        encode_control_frame(&ControlEnvelope::PassAlong(over)),
        Err(ControlCodecFault::SizeOverLimit(CONTROL_MESSAGE_SIZE_LIMIT - 16))
    );
}

#[test]
fn header_rejections() {
    let good = framed(1, RID, &[1]);
    assert!(decode_control_frame(&good).is_ok());
    assert_eq!(
        decode_control_frame(&vec![0u8; CONTROL_MESSAGE_SIZE_LIMIT + 1]),
        Err(ControlCodecFault::SizeOverLimit(CONTROL_MESSAGE_SIZE_LIMIT + 1))
    );
    assert_eq!(decode_control_frame(&good[..16]), Err(ControlCodecFault::Truncated));
    assert_eq!(decode_control_frame(&[]), Err(ControlCodecFault::Truncated));
    // Too short to hold a header: the size check wins over the magic check.
    assert_eq!(decode_control_frame(b"XXXX"), Err(ControlCodecFault::Truncated));
    let mut bad = good.clone();
    bad[0] = b'X';
    assert_eq!(decode_control_frame(&bad), Err(ControlCodecFault::WrongSignature));
    let mut bad = good.clone();
    bad[5] = 8;
    assert_eq!(
        decode_control_frame(&bad),
        Err(ControlCodecFault::WireRevisionSkew { local: 10, remote: 8 })
    );
    let mut bad = good.clone();
    bad[4] = 1;
    assert_eq!(
        decode_control_frame(&bad),
        Err(ControlCodecFault::WireRevisionSkew { local: 10, remote: 0x010a })
    );
    let mut bad = good.clone();
    bad.push(0);
    assert_eq!(
        decode_control_frame(&bad),
        Err(ControlCodecFault::BodyLengthMismatch { header: 1, present: 2 })
    );
    assert_eq!(
        decode_control_frame(&framed(7, 0, &[])),
        Err(ControlCodecFault::UnrecognizedKindByte(7))
    );
    assert_eq!(
        decode_control_frame(&framed(0xff, 0, &[1, 2])),
        Err(ControlCodecFault::UnrecognizedKindByte(0xff))
    );
}

#[test]
fn body_rejections() {
    let hello = Bytes::default().w(10).d(1).b(0).s("b").s("p");
    assert!(decode_control_frame(&framed(0, 0, &hello.0)).is_ok());
    let hello = Bytes::default().w(10).d(1).b(0).s("b").s("p").b(0);
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ControlCodecFault::MalformedContent));
    let hello = Bytes::default().w(10).d(1).b(3).s("b").s("p");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ControlCodecFault::MalformedContent));
    let hello = Bytes::default().w(10).d(1).b(0).b(2).raw(&[0xc3, 0x28]).s("p");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ControlCodecFault::TextNotUtf8));
    let hello = Bytes::default().w(10).d(1).b(0).b(65).raw(&[b'a'; 65]).s("p");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ControlCodecFault::SizeOverLimit(65)));
    let hello = Bytes::default().w(10).d(1).b(0).b(5).raw(b"ab");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ControlCodecFault::Truncated));

    assert_eq!(decode_control_frame(&framed(1, 0, &[])), Err(ControlCodecFault::Truncated));
    assert_eq!(decode_control_frame(&framed(1, 0, &[0x20])), Err(ControlCodecFault::UnrecognizedKindByte(0x20)));
    assert_eq!(decode_control_frame(&framed(1, 0, &[0])), Err(ControlCodecFault::UnrecognizedKindByte(0)));
    assert_eq!(decode_control_frame(&framed(1, 0, &[1, 0])), Err(ControlCodecFault::MalformedContent));
    let set = Bytes::default().b(5).s("m").s("d").b(2);
    assert_eq!(decode_control_frame(&framed(1, 0, &set.0)), Err(ControlCodecFault::MalformedContent));
    let fail = Bytes::default().b(15).raw(&id(1)).d(1).b(0);
    assert_eq!(decode_control_frame(&framed(1, 0, &fail.0)), Err(ControlCodecFault::MalformedContent));
    let fail = Bytes::default().b(15).raw(&id(1)).d(1).b(6);
    assert_eq!(decode_control_frame(&framed(1, 0, &fail.0)), Err(ControlCodecFault::MalformedContent));
    let fail = Bytes::default().b(15).raw(&id(1)).d(1).b(3).q(1);
    assert_eq!(decode_control_frame(&framed(1, 0, &fail.0)), Err(ControlCodecFault::Truncated));
    let join = Bytes::default().b(3).raw(&id(1)[..10]);
    assert_eq!(decode_control_frame(&framed(1, 0, &join.0)), Err(ControlCodecFault::Truncated));

    assert_eq!(decode_control_frame(&framed(2, 0, &[6])), Err(ControlCodecFault::UnrecognizedKindByte(6)));
    assert_eq!(decode_control_frame(&framed(2, 0, &[0x7f, 6])), Err(ControlCodecFault::MalformedContent));
    assert_eq!(decode_control_frame(&framed(2, 0, &[0x7f, 0])), Err(ControlCodecFault::MalformedContent));
    assert_eq!(decode_control_frame(&framed(2, 0, &[0x7f, 14, 1])), Err(ControlCodecFault::Truncated));
    assert_eq!(decode_control_frame(&framed(2, 0, &[10, 0])), Err(ControlCodecFault::MalformedContent));
    let list = Bytes::default().b(9).q(0).w(129);
    assert_eq!(decode_control_frame(&framed(2, 0, &list.0)), Err(ControlCodecFault::SizeOverLimit(129)));
    let list = Bytes::default()
        .b(9)
        .q(0)
        .w(1)
        .raw(&id(1))
        .s("n")
        .s("m")
        .s("d")
        .b(1)
        .b(2)
        .b(2)
        .b(0)
        .b(0)
        .q(0);
    assert_eq!(decode_control_frame(&framed(2, 0, &list.0)), Err(ControlCodecFault::MalformedContent));

    let mut view = sample_view_bytes().0;
    let count_at = view.len() - (16 + 4) - (16 + 3) - 1;
    assert_eq!(view[count_at], 2);
    view[count_at] = 19;
    assert_eq!(decode_control_frame(&framed(3, 0, &view)), Err(ControlCodecFault::SizeOverLimit(19)));
    let mut view = sample_view_bytes().0;
    view[16 + 16 + 8 + 4] = 3;
    assert_eq!(decode_control_frame(&framed(3, 0, &view)), Err(ControlCodecFault::MalformedContent));
    let mut view = sample_view_bytes().0;
    view[16 + 16 + 8 + 4 + 1] = 2;
    assert_eq!(decode_control_frame(&framed(3, 0, &view)), Err(ControlCodecFault::MalformedContent));

    let closed = Bytes::default().raw(&id(1)).d(0).b(4);
    assert_eq!(decode_control_frame(&framed(4, 0, &closed.0)), Err(ControlCodecFault::MalformedContent));
    let closed = Bytes::default().raw(&id(1)).d(0).b(0).b(0);
    assert_eq!(decode_control_frame(&framed(4, 0, &closed.0)), Err(ControlCodecFault::MalformedContent));

    assert_eq!(decode_control_frame(&framed(5, 0, &[7])), Err(ControlCodecFault::UnrecognizedKindByte(7)));
    assert_eq!(decode_control_frame(&framed(5, 0, &[0])), Err(ControlCodecFault::UnrecognizedKindByte(0)));
    let vote = Bytes::default().b(5).raw(&id(1)).b(0);
    assert_eq!(decode_control_frame(&framed(5, 0, &vote.0)), Err(ControlCodecFault::MalformedContent));
}

#[test]
fn encode_rejections() {
    let create = |name: &str, map: &str, ruleset: &str, seat_capacity: u8| {
        encode_control_frame(&ControlEnvelope::Request(ControlQuery {
            call_id: ASK_ID,
            body: QueryPayload::OpenLobby {
                name: name.to_owned(),
                map: map.to_owned(),
                mode: ruleset.to_owned(),
                seat_capacity,
                requires: AssetCoverageBits(0),
            },
        }))
    };
    assert!(create("n", "m", "d", 2).is_ok());
    assert!(create(&"n".repeat(48), &"m".repeat(64), &"d".repeat(48), 18).is_ok());
    for (n, m, d, p) in [
        ("", "m", "d", 2),
        ("n", "", "d", 2),
        ("n", "m", "", 2),
        ("n", "m", "d", 1),
        ("n", "m", "d", 19),
        (&"n".repeat(49) as &str, "m", "d", 2),
        ("n", &"m".repeat(65) as &str, "d", 2),
        ("n", "m", &"d".repeat(49) as &str, 2),
    ] {
        assert_eq!(create(n, m, d, p), Err(ControlCodecFault::MalformedContent), "{n:?} {m:?} {d:?} {p}");
    }
    let opts = |map: &str, ruleset: &str| {
        encode_control_frame(&ControlEnvelope::Request(ControlQuery {
            call_id: ASK_ID,
            body: QueryPayload::ConfigureLobby {
                map: map.to_owned(),
                mode: ruleset.to_owned(),
                open_to_join: true,
            },
        }))
    };
    assert_eq!(opts("", "d"), Err(ControlCodecFault::MalformedContent));
    assert_eq!(opts("m", &"d".repeat(49)), Err(ControlCodecFault::MalformedContent));
    let start = |map: &str, ruleset: &str| {
        encode_control_frame(&ControlEnvelope::Request(ControlQuery {
            call_id: ASK_ID,
            body: QueryPayload::BeginGame {
                map: map.to_owned(),
                mode: ruleset.to_owned(),
            },
        }))
    };
    assert_eq!(start(&"m".repeat(65), "d"), Err(ControlCodecFault::MalformedContent));
    assert_eq!(start("m", ""), Err(ControlCodecFault::MalformedContent));

    let hello = |build: &str, player: &str| {
        encode_control_frame(&ControlEnvelope::Hello(ControlGreeting {
            wire_rev: 10,
            match_wire_version: 1,
            role: PeerKindTag::Join,
            build: build.to_owned(),
            display_name: player.to_owned(),
        }))
    };
    assert!(hello("", "").is_ok());
    assert_eq!(hello(&"b".repeat(65), "p"), Err(ControlCodecFault::SizeOverLimit(65)));
    assert_eq!(hello("b", &"p".repeat(70)), Err(ControlCodecFault::SizeOverLimit(70)));

    let mut crowded = sample_view();
    crowded.members = (0..19).map(|i| RoomOccupantId(id(i))).collect();
    assert_eq!(
        encode_control_frame(&ControlEnvelope::LobbyOverview(crowded)),
        Err(ControlCodecFault::SizeOverLimit(19))
    );
    let mut bad = sample_view();
    bad.seat_capacity = 1;
    assert_eq!(encode_control_frame(&ControlEnvelope::LobbyOverview(bad)), Err(ControlCodecFault::MalformedContent));
    let mut long_name = sample_view();
    long_name.member_names.insert(long_name.host, "x".repeat(65));
    assert_eq!(
        encode_control_frame(&ControlEnvelope::LobbyOverview(long_name)),
        Err(ControlCodecFault::SizeOverLimit(65))
    );

    let (shut, playing, page) = (false, false, 0);
    let advert = RoomListing {
        id: ListingId(id(0)),
        name: "n".repeat(48),
        map: "m".repeat(64),
        mode: "d".repeat(48),
        players: 1,
        seat_capacity: 2,
        locked: shut,
        playing,
        requires: AssetCoverageBits(0),
        generation: page,
    };
    let list = |count: usize| {
        encode_control_frame(&ControlEnvelope::Response(ControlReply {
            call_id: ASK_ID,
            body: ReplyPayload::RoomList {
                generation: page,
                adverts: vec![advert.clone(); count],
            },
        }))
    };
    assert_eq!(list(129), Err(ControlCodecFault::SizeOverLimit(129)));
    // 128 full-size adverts do not fit one control message.
    let per_advert = 16 + 49 + 65 + 49 + 5 + 8;
    assert_eq!(list(128), Err(ControlCodecFault::SizeOverLimit(1 + 8 + 2 + 128 * per_advert)));
    assert!(list(80).is_ok());
    let mut long_advert = advert.clone();
    long_advert.name = "n".repeat(49);
    let enc = encode_control_frame(&ControlEnvelope::Response(ControlReply {
        call_id: ASK_ID,
        body: ReplyPayload::RoomList {
            generation: page,
            adverts: vec![long_advert],
        },
    }));
    assert_eq!(enc, Err(ControlCodecFault::SizeOverLimit(49)));
}

#[test]
fn stream_length_prefix() {
    assert_eq!(stream_frame_len([0, 0, 0, 0]), Err(ControlCodecFault::SizeOverLimit(0)));
    assert_eq!(stream_frame_len([0, 0, 0, 1]), Ok(1));
    assert_eq!(stream_frame_len([0, 0, 0x40, 0]), Ok(CONTROL_MESSAGE_SIZE_LIMIT));
    assert_eq!(
        stream_frame_len([0, 0, 0x40, 1]),
        Err(ControlCodecFault::SizeOverLimit(CONTROL_MESSAGE_SIZE_LIMIT + 1))
    );
    assert_eq!(
        stream_frame_len([0xff, 0xff, 0xff, 0xff]),
        Err(ControlCodecFault::SizeOverLimit(u32::MAX as usize))
    );
    assert_eq!(decode_control_frame(b"BO2"), Err(ControlCodecFault::Truncated));
}

#[test]
fn relay_datagrams() {
    let peer = RoomOccupantId(id(0x80));
    let p = [1u8, 2, 3];
    let cases: [(ForwardedPacket<'_>, Vec<u8>); 4] = [
        (ForwardedPacket::ReceiverToHost(&p), vec![10, 1, 1, 2, 3]),
        (
            ForwardedPacket::HostToParticipant {
                member_id: peer,
                payload: &p,
            },
            Bytes::default().b(10).b(2).raw(&peer.0).raw(&p).0,
        ),
        (
            ForwardedPacket::DirectoryToHost {
                member_id: peer,
                payload: &p,
            },
            Bytes::default().b(10).b(3).raw(&peer.0).raw(&p).0,
        ),
        (ForwardedPacket::DirectoryToParticipant(&p), vec![10, 4, 1, 2, 3]),
    ];
    for (dgram, expected) in cases {
        assert_eq!(write_relay(dgram.clone()).unwrap(), expected);
        assert_eq!(write_relay_stream(dgram.clone()).unwrap(), expected);
        assert_eq!(read_relay(&expected).unwrap(), dgram);
        assert_eq!(read_relay_stream(&expected).unwrap(), dgram);
    }
    assert_eq!(read_relay(&[10, 1]).unwrap(), ForwardedPacket::ReceiverToHost(&[]));
    assert_eq!(read_relay(&[10, 4]).unwrap(), ForwardedPacket::DirectoryToParticipant(&[]));

    let fits = vec![0u8; OPAQUE_BODY_BYTE_CAP];
    let big = vec![0u8; OPAQUE_BODY_BYTE_CAP + 1];
    assert!(write_relay(ForwardedPacket::ReceiverToHost(&fits)).is_ok());
    assert_eq!(
        write_relay(ForwardedPacket::ReceiverToHost(&big)),
        Err(ControlCodecFault::SizeOverLimit(OPAQUE_BODY_BYTE_CAP + 1))
    );
    assert!(write_relay_stream(ForwardedPacket::ReceiverToHost(&big)).is_ok());
    let huge = vec![0u8; TRANSFER_SIZE_LIMIT + 1];
    assert_eq!(
        write_relay_stream(ForwardedPacket::DirectoryToParticipant(&huge)),
        Err(ControlCodecFault::SizeOverLimit(TRANSFER_SIZE_LIMIT + 1))
    );
    let mut wire = vec![10, 2];
    wire.extend_from_slice(&peer.0);
    wire.extend_from_slice(&big);
    assert_eq!(read_relay(&wire), Err(ControlCodecFault::SizeOverLimit(OPAQUE_BODY_BYTE_CAP + 1)));
    assert!(read_relay_stream(&wire).is_ok());

    assert_eq!(read_relay(&[]), Err(ControlCodecFault::Truncated));
    assert_eq!(read_relay(&[9]), Err(ControlCodecFault::Truncated));
    assert_eq!(
        read_relay(&[8, 1, 0]),
        Err(ControlCodecFault::WireRevisionSkew { local: 10, remote: 8 })
    );
    let mut short = vec![10, 3];
    short.extend_from_slice(&[0; 15]);
    assert_eq!(read_relay(&short), Err(ControlCodecFault::Truncated));
    short.push(0);
    assert!(read_relay(&short).is_ok());
    assert_eq!(read_relay(&[10, 0, 1]), Err(ControlCodecFault::UnrecognizedKindByte(0)));
    assert_eq!(read_relay(&[10, 5, 1]), Err(ControlCodecFault::UnrecognizedKindByte(5)));
}

#[test]
fn view_helpers() {
    let view = sample_view();
    let ad = view.advert();
    assert_eq!(ad.id, view.lobby_number);
    assert_eq!(ad.name, "Room");
    assert_eq!(ad.map, "mp_raid");
    assert_eq!(ad.mode, "tdm");
    assert_eq!(ad.players, 2);
    assert_eq!(ad.seat_capacity, 12);
    assert!(!ad.locked);
    assert!(ad.playing);
    assert_eq!(ad.requires, AssetCoverageBits(8));
    assert_eq!(ad.generation, view.revision);
    assert!(ad.permits_join_ask());
    let shut = LobbyOverview {
        open_to_join: false,
        phase: LobbyStage::Gathering,
        ..sample_view()
    }
    .advert();
    assert!(shut.locked);
    assert!(!shut.playing);
    assert!(!shut.permits_join_ask());
    assert!(may_ask_to_join(false));
    assert!(!may_ask_to_join(true));
    assert!(view.contains(view.host));
    assert!(!view.contains(RoomOccupantId(id(0x99))));
    let mut packed = sample_view();
    packed.members = (0..300u32).map(|i| RoomOccupantId(id(i as u8))).collect();
    assert_eq!(packed.advert().players, ROOM_OCCUPANT_LIMIT);
    assert!(!LobbyStage::Gathering.is_playing());
    assert!(LobbyStage::Loading.is_playing());
    assert!(LobbyStage::Running.is_playing());
}

#[test]
fn messages() {
    use EntryRefusalReason;
    let shown = |why: EntryRefusalReason| why.to_string();
    assert_eq!("peer admission deadline", shown(EntryRefusalReason::CutoffInstant));
    assert_eq!("peer admission cancelled", shown(EntryRefusalReason::Cancelled));
    assert_eq!(
        "map content mismatch: host=00000000000000ab peer=0000000000000001",
        shown(EntryRefusalReason::LevelAssetHashes { host_digest: 0xab, peer_digest: 1 })
    );
    assert_eq!(
        "weapon table mismatch: host=0000000000000002 peer=ffffffffffffffff",
        shown(EntryRefusalReason::WeaponLedger { host_digest: 2, peer_digest: 0xffff_ffff_ffff_ffff })
    );
    assert_eq!(
        "class catalog mismatch: host=0000000000000003 peer=0000000000000004; use the same game build as the host",
        shown(EntryRefusalReason::LoadoutTableHashes { host_digest: 3, peer_digest: 4 })
    );
    let rows: [(ControlCodecFault, &str); 9] = [
        (ControlCodecFault::Truncated, "control message ends early"),
        (ControlCodecFault::WrongSignature, "control message has the wrong magic"),
        (
            ControlCodecFault::WireRevisionSkew { local: 10, remote: 3 },
            "wire revision mismatch: local=10 remote=3",
        ),
        (ControlCodecFault::UnrecognizedKindByte(42), "control tag 42 is not known"),
        (ControlCodecFault::SizeOverLimit(77), "control message of 77 bytes is over the limit"),
        (
            ControlCodecFault::BodyLengthMismatch { header: 4, present: 5 },
            "control body length mismatch: header=4 present=5",
        ),
        (ControlCodecFault::MalformedContent, "control message body is malformed"),
        (ControlCodecFault::TextNotUtf8, "control text is not valid UTF-8"),
        (ControlCodecFault::InvalidId, "id is not 32 lowercase hex digits"),
    ];
    for (fault, shown) in rows {
        assert_eq!(fault.to_string(), shown);
        let _: &dyn std::error::Error = &fault;
    }
    assert_eq!(DirectoryServiceFault::Full.to_string(), "Full");
    assert_eq!(
        DirectoryServiceFault::AbsentResource {
            requires: AssetCoverageBits(8),
            have: AssetCoverageBits(0),
        }
        .to_string(),
        "AbsentResource { requires: AssetCoverageBits(8), have: AssetCoverageBits(0) }"
    );
    let _: &dyn std::error::Error = &DirectoryServiceFault::Locked;
}
