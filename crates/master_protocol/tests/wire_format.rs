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

fn framed(kind: u8, request_id: u64, body: &[u8]) -> Vec<u8> {
    Bytes::default()
        .raw(b"BO2M")
        .w(9)
        .b(kind)
        .q(request_id)
        .w(body.len() as u16)
        .raw(body)
        .0
}

fn round_trip(frame: ControlFrame, expected: Vec<u8>) {
    let bytes = encode_control_frame(&frame).expect("encode");
    assert_eq!(bytes, expected, "encoded bytes for {frame:?}");
    assert_eq!(decode_control_frame(&bytes).expect("decode"), frame);
    let streamed = encode_stream_frame(&frame).expect("stream encode");
    assert_eq!(&streamed[..4], &(expected.len() as u32).to_be_bytes());
    assert_eq!(&streamed[4..], &expected[..]);
    assert_eq!(
        stream_frame_len(streamed[..4].try_into().unwrap()),
        Ok(expected.len())
    );
    assert_eq!(decode_stream_payload(&streamed[4..]).expect("payload"), frame);
}

fn request(body: RequestBody, expected_body: Vec<u8>) {
    round_trip(
        ControlFrame::Request(ControlRequest {
            request_id: RID,
            body,
        }),
        framed(1, RID, &expected_body),
    );
}

fn response(body: ResponseBody, expected_body: Vec<u8>) {
    round_trip(
        ControlFrame::Response(ControlResponse {
            request_id: RID,
            body,
        }),
        framed(2, RID, &expected_body),
    );
}

fn event(ev: PeerEvent, expected_body: Vec<u8>) {
    round_trip(ControlFrame::PeerEvent(ev), framed(5, 0, &expected_body));
}

fn sample_view() -> RoomView {
    let a = MemberId(id(0x40));
    let b = MemberId(id(0x60));
    let mut names = HashMap::new();
    names.insert(a, "Ann".to_owned());
    names.insert(b, "Bo".to_owned());
    RoomView {
        room_id: AdvertId(id(0x10)),
        name: "Room".to_owned(),
        host: a,
        members: vec![a, b],
        member_names: names,
        map: "mp_raid".to_owned(),
        mode: "tdm".to_owned(),
        joinable: true,
        revision: 0x0102_0304_0506_0708,
        epoch: 7,
        phase: RoomPhase::Loading,
        max_players: 12,
        requires: ContentFlags(0b1000),
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
    assert_eq!(PROTOCOL_VERSION, 9);
    assert_eq!(ALPN, b"bo2mp-master/9");
    assert_eq!(MAX_CONTROL_BYTES, 16384);
    assert_eq!(MAX_OPAQUE_PAYLOAD, 1100);
    assert_eq!(MAX_BOOTSTRAP_PAYLOAD, 262_144);
    assert_eq!(MAX_BOOTSTRAP_STREAM_BYTES, 262_162);
    assert_eq!(MAX_RELAY_UNI_STREAMS, 8);
    assert_eq!(MAX_CONCURRENT_BOOTSTRAP, 4);
    assert_eq!(MAX_SESSION_MEMBERS, 18);
    assert_eq!(MAX_ADVERT_NAME_BYTES, 48);
    assert_eq!(MAX_MAP_BYTES, 64);
    assert_eq!(MAX_MODE_BYTES, 24);
    assert_eq!(MAX_BUILD_BYTES, 64);
    assert_eq!(MAX_PLAYER_NAME_BYTES, 64);
    assert_eq!(MAX_LIST_ADVERTS, 128);
    assert_eq!(SESSION_IDLE, std::time::Duration::from_secs(8));
    assert_eq!(SESSION_KEEP_ALIVE, std::time::Duration::from_secs(1));
    assert_eq!(
        StatusResponse::CURRENT,
        StatusResponse {
            protocol_version: 9,
            max_opaque_payload: 1100,
            max_session_members: 18,
        }
    );
}

#[test]
fn channels() {
    assert_eq!(Channel::ALL, [Channel::Prod, Channel::Dev]);
    let rows = [
        (Channel::Prod, "prod", 4433, "bo2mp-master.service", "bo2mp-prod"),
        (Channel::Dev, "dev", 4434, "bo2mp-master-dev.service", "bo2mp-dev"),
    ];
    for (ch, name, port, unit, san) in rows {
        assert_eq!(ch.as_str(), name);
        assert_eq!(ch.to_string(), name);
        assert_eq!(ch.port(), port);
        assert_eq!(ch.unit(), unit);
        assert_eq!(ch.server_name(), san);
        assert_eq!(name.parse::<Channel>(), Ok(ch));
    }
    let err = "staging".parse::<Channel>().unwrap_err();
    assert_eq!(err.to_string(), "channel must be prod or dev");
    assert!("Prod".parse::<Channel>().is_err());
    let _: &dyn std::error::Error = &err;
}

#[test]
fn ids_print_and_parse_as_hex() {
    let a = AdvertId(id(0xa0));
    let text = a.to_string();
    assert_eq!(text, "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf");
    assert_eq!(text.parse::<AdvertId>(), Ok(a));
    assert_eq!(text.to_uppercase().parse::<AdvertId>(), Ok(a));
    let m = MemberId([0; 16]);
    assert_eq!(m.to_string(), "0".repeat(32));
    assert_eq!("0".repeat(32).parse::<MemberId>(), Ok(m));
    assert_eq!(AdvertId::default(), AdvertId([0; 16]));
    for bad in ["", "00", &"0".repeat(31), &"0".repeat(33), &format!("{}zz", "0".repeat(30))] {
        assert_eq!(bad.parse::<AdvertId>(), Err(ProtocolError::InvalidId), "{bad:?}");
        assert_eq!(bad.parse::<MemberId>(), Err(ProtocolError::InvalidId), "{bad:?}");
    }
    assert!(AdvertId(id(1)) < AdvertId(id(2)));
}

#[test]
fn content_flags() {
    let need = ContentFlags(0b1100);
    assert_eq!(need.missing_from(ContentFlags(0b0100)), ContentFlags(0b1000));
    assert!(need.missing_from(ContentFlags(0xff)).is_empty());
    assert!(ContentFlags::default().is_empty());
    assert!(!need.is_empty());
}

#[test]
fn hello_bytes() {
    let hello = ControlHello {
        protocol_version: 9,
        game_protocol: 0x0102_0304,
        role: EndpointRole::Host,
        build: "b1".to_owned(),
        player_name: "Al".to_owned(),
    };
    let body = Bytes::default().w(9).d(0x0102_0304).b(1).s("b1").s("Al").0;
    round_trip(ControlFrame::Hello(hello.clone()), framed(0, 0, &body));
    for (role, wire) in [(EndpointRole::Cli, 0), (EndpointRole::Host, 1), (EndpointRole::Join, 2)] {
        let frame = ControlFrame::Hello(ControlHello { role, ..hello.clone() });
        let body = Bytes::default().w(9).d(0x0102_0304).b(wire).s("b1").s("Al").0;
        round_trip(frame, framed(0, 0, &body));
    }
}

#[test]
fn request_bytes() {
    request(RequestBody::Status, vec![1]);
    request(
        RequestBody::CreateRoom {
            name: "Room".to_owned(),
            map: "mp_raid".to_owned(),
            mode: "tdm".to_owned(),
            max_players: 18,
            requires: ContentFlags(8),
        },
        Bytes::default().b(2).s("Room").s("mp_raid").s("tdm").b(18).b(8).0,
    );
    request(
        RequestBody::JoinRoom {
            room_id: AdvertId(id(0x10)),
            have: ContentFlags(12),
        },
        Bytes::default().b(3).raw(&id(0x10)).b(12).0,
    );
    request(RequestBody::LeaveRoom, vec![4]);
    request(
        RequestBody::SetOptions {
            map: "mp_slums".to_owned(),
            mode: "dom".to_owned(),
            joinable: false,
        },
        Bytes::default().b(5).s("mp_slums").s("dom").b(0).0,
    );
    request(
        RequestBody::SetOptions {
            map: "m".to_owned(),
            mode: "d".to_owned(),
            joinable: true,
        },
        Bytes::default().b(5).s("m").s("d").b(1).0,
    );
    request(
        RequestBody::StartMatch {
            map: "mp_raid".to_owned(),
            mode: "tdm".to_owned(),
        },
        Bytes::default().b(6).s("mp_raid").s("tdm").0,
    );
    request(
        RequestBody::EndMatch {
            room_id: AdvertId(id(0x20)),
            epoch: 0xdead_beef,
        },
        Bytes::default().b(7).raw(&id(0x20)).d(0xdead_beef).0,
    );
    request(RequestBody::CloseRoom, vec![8]);
    request(RequestBody::ListRooms, vec![9]);
    request(
        RequestBody::HostWorldReady {
            room_id: AdvertId(id(0x30)),
            epoch: 3,
            map: 0xa,
            weapons: 0xb,
            classes: 0xc,
        },
        Bytes::default().b(10).raw(&id(0x30)).d(3).q(0xa).q(0xb).q(0xc).0,
    );
    request(
        RequestBody::MapLoaded {
            epoch: 4,
            map: 1,
            weapons: 2,
            classes: 3,
        },
        Bytes::default().b(11).d(4).q(1).q(2).q(3).0,
    );
    request(
        RequestBody::BootstrapApplied {
            epoch: 5,
            bootstrap_id: 6,
            snapshot_seq: 7,
            connection: 8,
        },
        Bytes::default().b(12).d(5).d(6).d(7).q(8).0,
    );
    request(
        RequestBody::EnterMatch {
            member_id: MemberId(id(0x50)),
            epoch: 1,
            bootstrap_id: 2,
            connection_id: 3,
            client_id: 4,
        },
        Bytes::default().b(13).raw(&id(0x50)).d(1).d(2).q(3).d(4).0,
    );
    request(RequestBody::VoteToSkip, vec![14]);
    let reasons = [
        (AdmissionFailure::Deadline, Bytes::default().b(1)),
        (AdmissionFailure::Cancelled, Bytes::default().b(2)),
        (
            AdmissionFailure::MapContent { host: 1, peer: 2 },
            Bytes::default().b(3).q(1).q(2),
        ),
        (
            AdmissionFailure::WeaponRegistry { host: 3, peer: 4 },
            Bytes::default().b(4).q(3).q(4),
        ),
        (
            AdmissionFailure::ClassRegistry { host: 5, peer: 6 },
            Bytes::default().b(5).q(5).q(6),
        ),
    ];
    for (reason, tail) in reasons {
        request(
            RequestBody::AdmissionFailed {
                member_id: MemberId(id(0x70)),
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
        ResponseBody::Status(StatusResponse::CURRENT),
        Bytes::default().b(1).w(9).w(1100).b(18).0,
    );
    response(
        ResponseBody::RoomCreated {
            member_id: MemberId(id(0x40)),
            view: sample_view(),
        },
        Bytes::default().b(2).raw(&id(0x40)).raw(&sample_view_bytes().0).0,
    );
    response(
        ResponseBody::RoomJoined {
            member_id: MemberId(id(0x60)),
            view: sample_view(),
        },
        Bytes::default().b(3).raw(&id(0x60)).raw(&sample_view_bytes().0).0,
    );
    response(ResponseBody::RoomLeft, vec![4]);
    response(
        ResponseBody::RoomUpdated { view: sample_view() },
        Bytes::default().b(5).raw(&sample_view_bytes().0).0,
    );
    let advert = Advert {
        id: AdvertId(id(0x10)),
        name: "Room".to_owned(),
        map: "mp_raid".to_owned(),
        mode: "tdm".to_owned(),
        players: 3,
        max_players: 12,
        locked: true,
        in_match: false,
        requires: ContentFlags(8),
        generation: 77,
    };
    response(
        ResponseBody::RoomList {
            generation: 0x99,
            adverts: vec![advert.clone(), Advert { locked: false, in_match: true, ..advert }],
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
    response(
        ResponseBody::RoomList {
            generation: 0,
            adverts: Vec::new(),
        },
        Bytes::default().b(9).q(0).w(0).0,
    );
    response(ResponseBody::Ack, vec![10]);
    let errors = [
        (ServiceError::Malformed, vec![1]),
        (ServiceError::InvalidAdvert, vec![2]),
        (ServiceError::AdvertCapacity, vec![3]),
        (ServiceError::AlreadyHosting, vec![4]),
        (ServiceError::UnknownAdvert, vec![5]),
        (ServiceError::Full, vec![7]),
        (ServiceError::Locked, vec![8]),
        (ServiceError::NotHost, vec![11]),
        (ServiceError::NotMember, vec![13]),
        (
            ServiceError::MissingContent {
                requires: ContentFlags(12),
                have: ContentFlags(4),
            },
            vec![14, 12, 4],
        ),
    ];
    for (err, tail) in errors {
        response(ResponseBody::Error(err), Bytes::default().b(0x7f).raw(&tail).0);
    }
}

#[test]
fn room_view_frame_and_default_names() {
    round_trip(ControlFrame::RoomView(sample_view()), framed(3, 0, &sample_view_bytes().0));
    // A member without a stored name goes out as "Player" and comes back named.
    let mut view = sample_view();
    view.member_names.clear();
    let bytes = encode_control_frame(&ControlFrame::RoomView(view.clone())).unwrap();
    let ControlFrame::RoomView(back) = decode_control_frame(&bytes).unwrap() else {
        panic!("not a room view");
    };
    assert_eq!(back.member_names.len(), 2);
    assert!(back.member_names.values().all(|n| n == "Player"));
    assert_eq!(back.members, view.members);
    for (phase, wire) in [(RoomPhase::Gathering, 0u8), (RoomPhase::Loading, 1), (RoomPhase::Running, 2)] {
        let v = RoomView { phase, ..sample_view() };
        let enc = encode_control_frame(&ControlFrame::RoomView(v.clone())).unwrap();
        assert_eq!(enc[17 + 16 + 16 + 8 + 4], wire);
        assert_eq!(decode_control_frame(&enc).unwrap(), ControlFrame::RoomView(v));
    }
}

#[test]
fn closed_bytes() {
    for (reason, wire) in [
        (SessionCloseReason::HostLeft, 0u8),
        (SessionCloseReason::LeaseExpired, 1),
        (SessionCloseReason::Overload, 2),
    ] {
        round_trip(
            ControlFrame::Closed {
                room_id: AdvertId(id(0x10)),
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
        PeerEvent::HostWorldReady {
            epoch: 1,
            map: 2,
            weapons: 3,
            classes: 4,
        },
        Bytes::default().b(1).d(1).q(2).q(3).q(4).0,
    );
    event(
        PeerEvent::MapLoaded {
            member_id: MemberId(id(0x50)),
            epoch: 1,
            map: 2,
            weapons: 3,
            classes: 4,
        },
        Bytes::default().b(2).raw(&id(0x50)).d(1).q(2).q(3).q(4).0,
    );
    event(
        PeerEvent::BootstrapApplied {
            member_id: MemberId(id(0x50)),
            epoch: 1,
            bootstrap_id: 2,
            snapshot_seq: 3,
            connection: 4,
        },
        Bytes::default().b(3).raw(&id(0x50)).d(1).d(2).d(3).q(4).0,
    );
    event(
        PeerEvent::EnterMatch {
            member_id: MemberId(id(0x50)),
            epoch: 1,
            bootstrap_id: 2,
            connection_id: 3,
            client_id: 4,
        },
        Bytes::default().b(4).raw(&id(0x50)).d(1).d(2).q(3).d(4).0,
    );
    event(
        PeerEvent::VoteToSkip {
            member_id: MemberId(id(0x50)),
        },
        Bytes::default().b(5).raw(&id(0x50)).0,
    );
    event(
        PeerEvent::AdmissionFailed {
            member_id: MemberId(id(0x50)),
            epoch: 6,
            reason: AdmissionFailure::ClassRegistry { host: 7, peer: 8 },
        },
        Bytes::default().b(6).raw(&id(0x50)).d(6).b(5).q(7).q(8).0,
    );
}

#[test]
fn relay_frame_inside_control_stream() {
    let payload = vec![0xee; 40];
    round_trip(ControlFrame::Relay(payload.clone()), framed(6, 0, &payload));
    round_trip(ControlFrame::Relay(Vec::new()), framed(6, 0, &[]));
    let largest = vec![1u8; MAX_CONTROL_BYTES - 17];
    assert!(encode_control_frame(&ControlFrame::Relay(largest)).is_ok());
    let over = vec![1u8; MAX_CONTROL_BYTES - 16];
    assert_eq!(
        encode_control_frame(&ControlFrame::Relay(over)),
        Err(ProtocolError::LengthExceeded(MAX_CONTROL_BYTES - 16))
    );
}

#[test]
fn header_rejections() {
    let good = framed(1, RID, &[1]);
    assert!(decode_control_frame(&good).is_ok());
    assert_eq!(
        decode_control_frame(&vec![0u8; MAX_CONTROL_BYTES + 1]),
        Err(ProtocolError::LengthExceeded(MAX_CONTROL_BYTES + 1))
    );
    assert_eq!(decode_control_frame(&good[..16]), Err(ProtocolError::Truncated));
    assert_eq!(decode_control_frame(&[]), Err(ProtocolError::Truncated));
    // Too short to hold a header: the size check wins over the magic check.
    assert_eq!(decode_control_frame(b"XXXX"), Err(ProtocolError::Truncated));
    let mut bad = good.clone();
    bad[0] = b'X';
    assert_eq!(decode_control_frame(&bad), Err(ProtocolError::BadMagic));
    let mut bad = good.clone();
    bad[5] = 8;
    assert_eq!(
        decode_control_frame(&bad),
        Err(ProtocolError::VersionMismatch { ours: 9, theirs: 8 })
    );
    let mut bad = good.clone();
    bad[4] = 1;
    assert_eq!(
        decode_control_frame(&bad),
        Err(ProtocolError::VersionMismatch { ours: 9, theirs: 0x0109 })
    );
    let mut bad = good.clone();
    bad.push(0);
    assert_eq!(
        decode_control_frame(&bad),
        Err(ProtocolError::LengthMismatch { declared: 1, actual: 2 })
    );
    assert_eq!(
        decode_control_frame(&framed(7, 0, &[])),
        Err(ProtocolError::UnknownTag(7))
    );
    assert_eq!(
        decode_control_frame(&framed(0xff, 0, &[1, 2])),
        Err(ProtocolError::UnknownTag(0xff))
    );
}

#[test]
fn body_rejections() {
    let hello = Bytes::default().w(9).d(1).b(0).s("b").s("p");
    assert!(decode_control_frame(&framed(0, 0, &hello.0)).is_ok());
    let hello = Bytes::default().w(9).d(1).b(0).s("b").s("p").b(0);
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ProtocolError::InvalidBody));
    let hello = Bytes::default().w(9).d(1).b(3).s("b").s("p");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ProtocolError::InvalidBody));
    let hello = Bytes::default().w(9).d(1).b(0).b(2).raw(&[0xc3, 0x28]).s("p");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ProtocolError::InvalidUtf8));
    let hello = Bytes::default().w(9).d(1).b(0).b(65).raw(&[b'a'; 65]).s("p");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ProtocolError::LengthExceeded(65)));
    let hello = Bytes::default().w(9).d(1).b(0).b(5).raw(b"ab");
    assert_eq!(decode_control_frame(&framed(0, 0, &hello.0)), Err(ProtocolError::Truncated));

    assert_eq!(decode_control_frame(&framed(1, 0, &[])), Err(ProtocolError::Truncated));
    assert_eq!(decode_control_frame(&framed(1, 0, &[0x20])), Err(ProtocolError::UnknownTag(0x20)));
    assert_eq!(decode_control_frame(&framed(1, 0, &[0])), Err(ProtocolError::UnknownTag(0)));
    assert_eq!(decode_control_frame(&framed(1, 0, &[1, 0])), Err(ProtocolError::InvalidBody));
    let set = Bytes::default().b(5).s("m").s("d").b(2);
    assert_eq!(decode_control_frame(&framed(1, 0, &set.0)), Err(ProtocolError::InvalidBody));
    let fail = Bytes::default().b(15).raw(&id(1)).d(1).b(0);
    assert_eq!(decode_control_frame(&framed(1, 0, &fail.0)), Err(ProtocolError::InvalidBody));
    let fail = Bytes::default().b(15).raw(&id(1)).d(1).b(6);
    assert_eq!(decode_control_frame(&framed(1, 0, &fail.0)), Err(ProtocolError::InvalidBody));
    let fail = Bytes::default().b(15).raw(&id(1)).d(1).b(3).q(1);
    assert_eq!(decode_control_frame(&framed(1, 0, &fail.0)), Err(ProtocolError::Truncated));
    let join = Bytes::default().b(3).raw(&id(1)[..10]);
    assert_eq!(decode_control_frame(&framed(1, 0, &join.0)), Err(ProtocolError::Truncated));

    assert_eq!(decode_control_frame(&framed(2, 0, &[6])), Err(ProtocolError::UnknownTag(6)));
    assert_eq!(decode_control_frame(&framed(2, 0, &[0x7f, 6])), Err(ProtocolError::InvalidBody));
    assert_eq!(decode_control_frame(&framed(2, 0, &[0x7f, 0])), Err(ProtocolError::InvalidBody));
    assert_eq!(decode_control_frame(&framed(2, 0, &[0x7f, 14, 1])), Err(ProtocolError::Truncated));
    assert_eq!(decode_control_frame(&framed(2, 0, &[10, 0])), Err(ProtocolError::InvalidBody));
    let list = Bytes::default().b(9).q(0).w(129);
    assert_eq!(decode_control_frame(&framed(2, 0, &list.0)), Err(ProtocolError::LengthExceeded(129)));
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
    assert_eq!(decode_control_frame(&framed(2, 0, &list.0)), Err(ProtocolError::InvalidBody));

    let mut view = sample_view_bytes().0;
    let count_at = view.len() - (16 + 4) - (16 + 3) - 1;
    assert_eq!(view[count_at], 2);
    view[count_at] = 19;
    assert_eq!(decode_control_frame(&framed(3, 0, &view)), Err(ProtocolError::LengthExceeded(19)));
    let mut view = sample_view_bytes().0;
    view[16 + 16 + 8 + 4] = 3;
    assert_eq!(decode_control_frame(&framed(3, 0, &view)), Err(ProtocolError::InvalidBody));
    let mut view = sample_view_bytes().0;
    view[16 + 16 + 8 + 4 + 1] = 2;
    assert_eq!(decode_control_frame(&framed(3, 0, &view)), Err(ProtocolError::InvalidBody));

    let closed = Bytes::default().raw(&id(1)).d(0).b(3);
    assert_eq!(decode_control_frame(&framed(4, 0, &closed.0)), Err(ProtocolError::InvalidBody));
    let closed = Bytes::default().raw(&id(1)).d(0).b(0).b(0);
    assert_eq!(decode_control_frame(&framed(4, 0, &closed.0)), Err(ProtocolError::InvalidBody));

    assert_eq!(decode_control_frame(&framed(5, 0, &[7])), Err(ProtocolError::UnknownTag(7)));
    assert_eq!(decode_control_frame(&framed(5, 0, &[0])), Err(ProtocolError::UnknownTag(0)));
    let vote = Bytes::default().b(5).raw(&id(1)).b(0);
    assert_eq!(decode_control_frame(&framed(5, 0, &vote.0)), Err(ProtocolError::InvalidBody));
}

#[test]
fn encode_rejections() {
    let create = |name: &str, map: &str, mode: &str, max_players: u8| {
        encode_control_frame(&ControlFrame::Request(ControlRequest {
            request_id: 1,
            body: RequestBody::CreateRoom {
                name: name.to_owned(),
                map: map.to_owned(),
                mode: mode.to_owned(),
                max_players,
                requires: ContentFlags(0),
            },
        }))
    };
    assert!(create("n", "m", "d", 2).is_ok());
    assert!(create(&"n".repeat(48), &"m".repeat(64), &"d".repeat(24), 18).is_ok());
    for (n, m, d, p) in [
        ("", "m", "d", 2),
        ("n", "", "d", 2),
        ("n", "m", "", 2),
        ("n", "m", "d", 1),
        ("n", "m", "d", 19),
        (&"n".repeat(49) as &str, "m", "d", 2),
        ("n", &"m".repeat(65) as &str, "d", 2),
        ("n", "m", &"d".repeat(25) as &str, 2),
    ] {
        assert_eq!(create(n, m, d, p), Err(ProtocolError::InvalidBody), "{n:?} {m:?} {d:?} {p}");
    }
    let opts = |map: &str, mode: &str| {
        encode_control_frame(&ControlFrame::Request(ControlRequest {
            request_id: 1,
            body: RequestBody::SetOptions {
                map: map.to_owned(),
                mode: mode.to_owned(),
                joinable: true,
            },
        }))
    };
    assert_eq!(opts("", "d"), Err(ProtocolError::InvalidBody));
    assert_eq!(opts("m", &"d".repeat(25)), Err(ProtocolError::InvalidBody));
    let start = |map: &str, mode: &str| {
        encode_control_frame(&ControlFrame::Request(ControlRequest {
            request_id: 1,
            body: RequestBody::StartMatch {
                map: map.to_owned(),
                mode: mode.to_owned(),
            },
        }))
    };
    assert_eq!(start(&"m".repeat(65), "d"), Err(ProtocolError::InvalidBody));
    assert_eq!(start("m", ""), Err(ProtocolError::InvalidBody));

    let hello = |build: &str, player: &str| {
        encode_control_frame(&ControlFrame::Hello(ControlHello {
            protocol_version: 9,
            game_protocol: 1,
            role: EndpointRole::Join,
            build: build.to_owned(),
            player_name: player.to_owned(),
        }))
    };
    assert!(hello("", "").is_ok());
    assert_eq!(hello(&"b".repeat(65), "p"), Err(ProtocolError::LengthExceeded(65)));
    assert_eq!(hello("b", &"p".repeat(70)), Err(ProtocolError::LengthExceeded(70)));

    let mut crowded = sample_view();
    crowded.members = (0..19).map(|i| MemberId(id(i))).collect();
    assert_eq!(
        encode_control_frame(&ControlFrame::RoomView(crowded)),
        Err(ProtocolError::LengthExceeded(19))
    );
    let mut bad = sample_view();
    bad.max_players = 1;
    assert_eq!(encode_control_frame(&ControlFrame::RoomView(bad)), Err(ProtocolError::InvalidBody));
    let mut long_name = sample_view();
    long_name.member_names.insert(long_name.host, "x".repeat(65));
    assert_eq!(
        encode_control_frame(&ControlFrame::RoomView(long_name)),
        Err(ProtocolError::LengthExceeded(65))
    );

    let advert = Advert {
        id: AdvertId(id(0)),
        name: "n".repeat(48),
        map: "m".repeat(64),
        mode: "d".repeat(24),
        players: 1,
        max_players: 2,
        locked: false,
        in_match: false,
        requires: ContentFlags(0),
        generation: 0,
    };
    let list = |count: usize| {
        encode_control_frame(&ControlFrame::Response(ControlResponse {
            request_id: 1,
            body: ResponseBody::RoomList {
                generation: 0,
                adverts: vec![advert.clone(); count],
            },
        }))
    };
    assert_eq!(list(129), Err(ProtocolError::LengthExceeded(129)));
    // 128 full-size adverts do not fit one control message.
    let per_advert = 16 + 49 + 65 + 25 + 5 + 8;
    assert_eq!(list(128), Err(ProtocolError::LengthExceeded(1 + 8 + 2 + 128 * per_advert)));
    assert!(list(90).is_ok());
    let mut long_advert = advert.clone();
    long_advert.name = "n".repeat(49);
    let enc = encode_control_frame(&ControlFrame::Response(ControlResponse {
        request_id: 1,
        body: ResponseBody::RoomList {
            generation: 0,
            adverts: vec![long_advert],
        },
    }));
    assert_eq!(enc, Err(ProtocolError::LengthExceeded(49)));
}

#[test]
fn stream_length_prefix() {
    assert_eq!(stream_frame_len([0, 0, 0, 0]), Err(ProtocolError::LengthExceeded(0)));
    assert_eq!(stream_frame_len([0, 0, 0, 1]), Ok(1));
    assert_eq!(stream_frame_len([0, 0, 0x40, 0]), Ok(MAX_CONTROL_BYTES));
    assert_eq!(
        stream_frame_len([0, 0, 0x40, 1]),
        Err(ProtocolError::LengthExceeded(MAX_CONTROL_BYTES + 1))
    );
    assert_eq!(
        stream_frame_len([0xff, 0xff, 0xff, 0xff]),
        Err(ProtocolError::LengthExceeded(u32::MAX as usize))
    );
    assert_eq!(decode_stream_payload(b"BO2"), Err(ProtocolError::Truncated));
}

#[test]
fn relay_datagrams() {
    let member = MemberId(id(0x80));
    let p = [1u8, 2, 3];
    let cases: [(RelayDatagram<'_>, Vec<u8>); 4] = [
        (RelayDatagram::ClientToHost(&p), vec![9, 1, 1, 2, 3]),
        (
            RelayDatagram::HostToMember {
                member_id: member,
                payload: &p,
            },
            Bytes::default().b(9).b(2).raw(&member.0).raw(&p).0,
        ),
        (
            RelayDatagram::ServiceToHost {
                member_id: member,
                payload: &p,
            },
            Bytes::default().b(9).b(3).raw(&member.0).raw(&p).0,
        ),
        (RelayDatagram::ServiceToMember(&p), vec![9, 4, 1, 2, 3]),
    ];
    for (dgram, expected) in cases {
        assert_eq!(encode_relay(dgram.clone()).unwrap(), expected);
        assert_eq!(encode_relay_stream(dgram.clone()).unwrap(), expected);
        assert_eq!(decode_relay(&expected).unwrap(), dgram);
        assert_eq!(decode_relay_stream(&expected).unwrap(), dgram);
    }
    assert_eq!(decode_relay(&[9, 1]).unwrap(), RelayDatagram::ClientToHost(&[]));
    assert_eq!(decode_relay(&[9, 4]).unwrap(), RelayDatagram::ServiceToMember(&[]));

    let fits = vec![0u8; MAX_OPAQUE_PAYLOAD];
    let big = vec![0u8; MAX_OPAQUE_PAYLOAD + 1];
    assert!(encode_relay(RelayDatagram::ClientToHost(&fits)).is_ok());
    assert_eq!(
        encode_relay(RelayDatagram::ClientToHost(&big)),
        Err(ProtocolError::LengthExceeded(MAX_OPAQUE_PAYLOAD + 1))
    );
    assert!(encode_relay_stream(RelayDatagram::ClientToHost(&big)).is_ok());
    let huge = vec![0u8; MAX_BOOTSTRAP_PAYLOAD + 1];
    assert_eq!(
        encode_relay_stream(RelayDatagram::ServiceToMember(&huge)),
        Err(ProtocolError::LengthExceeded(MAX_BOOTSTRAP_PAYLOAD + 1))
    );
    let mut wire = vec![9, 2];
    wire.extend_from_slice(&member.0);
    wire.extend_from_slice(&big);
    assert_eq!(decode_relay(&wire), Err(ProtocolError::LengthExceeded(MAX_OPAQUE_PAYLOAD + 1)));
    assert!(decode_relay_stream(&wire).is_ok());

    assert_eq!(decode_relay(&[]), Err(ProtocolError::Truncated));
    assert_eq!(decode_relay(&[9]), Err(ProtocolError::Truncated));
    assert_eq!(
        decode_relay(&[8, 1, 0]),
        Err(ProtocolError::VersionMismatch { ours: 9, theirs: 8 })
    );
    let mut short = vec![9, 3];
    short.extend_from_slice(&[0; 15]);
    assert_eq!(decode_relay(&short), Err(ProtocolError::Truncated));
    short.push(0);
    assert!(decode_relay(&short).is_ok());
    assert_eq!(decode_relay(&[9, 0, 1]), Err(ProtocolError::UnknownTag(0)));
    assert_eq!(decode_relay(&[9, 5, 1]), Err(ProtocolError::UnknownTag(5)));
}

#[test]
fn view_helpers() {
    let view = sample_view();
    let ad = view.advert();
    assert_eq!(ad.id, view.room_id);
    assert_eq!(ad.name, "Room");
    assert_eq!(ad.map, "mp_raid");
    assert_eq!(ad.mode, "tdm");
    assert_eq!(ad.players, 2);
    assert_eq!(ad.max_players, 12);
    assert!(!ad.locked);
    assert!(ad.in_match);
    assert_eq!(ad.requires, ContentFlags(8));
    assert_eq!(ad.generation, view.revision);
    assert!(ad.allows_join_request());
    let shut = RoomView {
        joinable: false,
        phase: RoomPhase::Gathering,
        ..sample_view()
    }
    .advert();
    assert!(shut.locked);
    assert!(!shut.in_match);
    assert!(!shut.allows_join_request());
    assert!(request_join_allowed(false));
    assert!(!request_join_allowed(true));
    assert!(view.contains(view.host));
    assert!(!view.contains(MemberId(id(0x99))));
    let mut packed = sample_view();
    packed.members = (0..300u32).map(|i| MemberId(id(i as u8))).collect();
    assert_eq!(packed.advert().players, MAX_SESSION_MEMBERS);
    assert!(!RoomPhase::Gathering.in_match());
    assert!(RoomPhase::Loading.in_match());
    assert!(RoomPhase::Running.in_match());
}

#[test]
fn messages() {
    assert_eq!(AdmissionFailure::Deadline.to_string(), "peer admission deadline");
    assert_eq!(AdmissionFailure::Cancelled.to_string(), "peer admission cancelled");
    assert_eq!(
        AdmissionFailure::MapContent { host: 0xab, peer: 1 }.to_string(),
        "map content mismatch: host=00000000000000ab peer=0000000000000001"
    );
    assert_eq!(
        AdmissionFailure::WeaponRegistry { host: 2, peer: 0xffff_ffff_ffff_ffff }.to_string(),
        "weapon table mismatch: host=0000000000000002 peer=ffffffffffffffff"
    );
    assert_eq!(
        AdmissionFailure::ClassRegistry { host: 3, peer: 4 }.to_string(),
        "class catalog mismatch: host=0000000000000003 peer=0000000000000004; use the same game build as the host"
    );
    let rows: [(ProtocolError, &str); 9] = [
        (ProtocolError::Truncated, "control message is truncated"),
        (ProtocolError::BadMagic, "control message has invalid magic"),
        (
            ProtocolError::VersionMismatch { ours: 9, theirs: 3 },
            "protocol version ours=9 theirs=3",
        ),
        (ProtocolError::UnknownTag(42), "unknown control tag 42"),
        (ProtocolError::LengthExceeded(77), "control message length 77 exceeds limit"),
        (
            ProtocolError::LengthMismatch { declared: 4, actual: 5 },
            "control body length declared=4 actual=5",
        ),
        (ProtocolError::InvalidBody, "control message body is invalid"),
        (ProtocolError::InvalidUtf8, "control string is not UTF-8"),
        (ProtocolError::InvalidId, "identifier is not canonical lowercase hex"),
    ];
    for (err, text) in rows {
        assert_eq!(err.to_string(), text);
        let _: &dyn std::error::Error = &err;
    }
    assert_eq!(ServiceError::Full.to_string(), "Full");
    assert_eq!(
        ServiceError::MissingContent {
            requires: ContentFlags(8),
            have: ContentFlags(0),
        }
        .to_string(),
        "MissingContent { requires: ContentFlags(8), have: ContentFlags(0) }"
    );
    let _: &dyn std::error::Error = &ServiceError::Locked;
}
