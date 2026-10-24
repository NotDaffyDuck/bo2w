//! The lobby's bookkeeping: who is connected, which rooms exist and who sits where.
//! Nothing here touches a socket. Every change hands back the frames it owes to
//! other connections, and the QUIC side delivers them.

use std::collections::HashMap;

use master_protocol::{
    AdmissionFailure, Advert, AdvertId, ContentFlags, ControlFrame, MemberId, PeerEvent,
    RelayDatagram, RequestBody, ResponseBody, RoomPhase, RoomView, ServiceError,
    SessionCloseReason, StatusResponse, MAX_LIST_ADVERTS,
};

/// The number the accept loop gives each connection, starting at 1.
pub type Link = u64;

/// Frames owed to connections, in the order they should go out.
pub type Outbox = Vec<(Link, ControlFrame)>;

/// The most rooms open at once; one more is refused.
pub const ROOM_LIMIT: usize = 10_000;
/// Longest display name kept, in characters.
const NAME_CHARS: usize = 16;
/// The name shown for a player who sent none.
const FALLBACK_NAME: &str = "Player";

/// A player's chosen name as other players will see it: trimmed, control
/// characters dropped, at most 16 characters, never empty.
pub fn display_name(raw: &str) -> String {
    let kept: String = raw
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(NAME_CHARS)
        .collect();
    if kept.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        kept
    }
}

/// One open room and how its member ids map to connections.
struct Table {
    view: RoomView,
    host_link: Link,
    member_by_link: HashMap<Link, MemberId>,
    link_by_member: HashMap<MemberId, Link>,
}

impl Table {
    /// Records a change the members must see.
    fn touch(&mut self) {
        self.view.revision = self.view.revision.wrapping_add(1).max(1);
    }

    fn view_for_all(&self) -> Outbox {
        let frame = ControlFrame::RoomView(self.view.clone());
        self.member_by_link.keys().map(|&link| (link, frame.clone())).collect()
    }

    fn event_for_all(&self, event: &PeerEvent) -> Outbox {
        let frame = ControlFrame::PeerEvent(event.clone());
        self.member_by_link.keys().map(|&link| (link, frame.clone())).collect()
    }

    /// The host's connection is past match start for this exact match.
    fn running(&self, room_id: AdvertId, epoch: u32) -> bool {
        room_id == self.view.room_id
            && epoch != 0
            && epoch == self.view.epoch
            && self.view.phase.in_match()
    }
}

/// Where a relayed packet goes next and what it says when it gets there.
#[derive(Debug, PartialEq, Eq)]
pub struct Hop<'a> {
    pub to: Link,
    /// True when `to` is the room's host (a missing host connection is `NotHost`).
    pub to_host: bool,
    pub packet: RelayDatagram<'a>,
}

/// Every room and every connected player.
pub struct Lobby {
    names: HashMap<Link, String>,
    tables: HashMap<AdvertId, Table>,
    seated_at: HashMap<Link, AdvertId>,
    /// Goes up on any change to any room; the room list reports it.
    generation: u64,
    fresh_id: Box<dyn FnMut() -> [u8; 16] + Send>,
}

type Answer = (ResponseBody, Outbox);

fn refuse(error: ServiceError) -> Answer {
    (ResponseBody::Error(error), Vec::new())
}

impl Lobby {
    /// `fresh_id` supplies the random room and member ids.
    pub fn new(fresh_id: impl FnMut() -> [u8; 16] + Send + 'static) -> Self {
        Self {
            names: HashMap::new(),
            tables: HashMap::new(),
            seated_at: HashMap::new(),
            generation: 0,
            fresh_id: Box::new(fresh_id),
        }
    }

    /// A connection said hello with this (already cleaned) name.
    pub fn arrive(&mut self, link: Link, name: String) {
        self.names.insert(link, name);
    }

    /// A connection is gone. A host takes its room with it; a member just leaves.
    pub fn depart(&mut self, link: Link) -> Outbox {
        self.names.remove(&link);
        let Some(&room_id) = self.seated_at.get(&link) else {
            return Vec::new();
        };
        match self.tables.get(&room_id) {
            None => {
                self.seated_at.remove(&link);
                Vec::new()
            }
            Some(table) if table.host_link == link => {
                self.close(room_id, SessionCloseReason::HostLeft)
            }
            Some(_) => self.unseat(link),
        }
    }

    /// The room and member id of a seated connection.
    pub fn seat(&self, link: Link) -> Result<(AdvertId, MemberId), ServiceError> {
        let room_id = *self.seated_at.get(&link).ok_or(ServiceError::NotMember)?;
        let table = self.tables.get(&room_id).ok_or(ServiceError::UnknownAdvert)?;
        let member = *table.member_by_link.get(&link).ok_or(ServiceError::NotMember)?;
        Ok((room_id, member))
    }

    /// The room this connection hosts.
    fn hosted(&self, link: Link) -> Result<AdvertId, ServiceError> {
        let room_id = *self.seated_at.get(&link).ok_or(ServiceError::NotMember)?;
        let table = self.tables.get(&room_id).ok_or(ServiceError::UnknownAdvert)?;
        if table.host_link == link {
            Ok(room_id)
        } else {
            Err(ServiceError::NotHost)
        }
    }

    fn hosted_table(&mut self, link: Link) -> Result<&mut Table, ServiceError> {
        let room_id = self.hosted(link)?;
        self.tables.get_mut(&room_id).ok_or(ServiceError::UnknownAdvert)
    }

    /// Where a packet from `link` must go: a member's packet to its host, a host's
    /// packet to the member it names. Packets already marked as from the service
    /// are refused.
    pub fn route<'a>(
        &self,
        link: Link,
        packet: &RelayDatagram<'a>,
    ) -> Result<Hop<'a>, ServiceError> {
        match *packet {
            RelayDatagram::ClientToHost(payload) => {
                let (room_id, member_id) = self.seat(link)?;
                let table = self.tables.get(&room_id).ok_or(ServiceError::UnknownAdvert)?;
                Ok(Hop {
                    to: table.host_link,
                    to_host: true,
                    packet: RelayDatagram::ServiceToHost { member_id, payload },
                })
            }
            RelayDatagram::HostToMember { member_id, payload } => {
                let room_id = self.hosted(link)?;
                let table = self.tables.get(&room_id).ok_or(ServiceError::UnknownAdvert)?;
                let to = *table
                    .link_by_member
                    .get(&member_id)
                    .ok_or(ServiceError::NotMember)?;
                Ok(Hop {
                    to,
                    to_host: false,
                    packet: RelayDatagram::ServiceToMember(payload),
                })
            }
            RelayDatagram::ServiceToHost { .. } | RelayDatagram::ServiceToMember(_) => {
                Err(ServiceError::Malformed)
            }
        }
    }

    /// Carries out one request. The answer goes to `link` after the outbox.
    pub fn answer(&mut self, link: Link, request: RequestBody) -> Answer {
        use RequestBody as R;
        match request {
            R::Status => (ResponseBody::Status(StatusResponse::CURRENT), Vec::new()),
            R::CreateRoom {
                name,
                map,
                mode,
                max_players,
                requires,
            } => self.open(link, name, map, mode, max_players, requires),
            R::JoinRoom { room_id, have } => self.join(link, room_id, have),
            R::LeaveRoom => self.leave(link),
            R::SetOptions {
                map,
                mode,
                joinable,
            } => self.configure(link, map, mode, Some(joinable)),
            R::StartMatch { map, mode } => self.configure(link, map, mode, None),
            R::EndMatch { room_id, epoch } => self.phase_change(link, room_id, epoch, None),
            R::CloseRoom => match self.hosted(link) {
                Ok(room_id) => (
                    ResponseBody::Ack,
                    self.close(room_id, SessionCloseReason::HostLeft),
                ),
                Err(error) => refuse(error),
            },
            R::ListRooms => (self.listing(), Vec::new()),
            R::HostWorldReady {
                room_id,
                epoch,
                map,
                weapons,
                classes,
            } => {
                let ready = PeerEvent::HostWorldReady {
                    epoch,
                    map,
                    weapons,
                    classes,
                };
                self.phase_change(link, room_id, epoch, Some(ready))
            }
            R::MapLoaded {
                epoch,
                map,
                weapons,
                classes,
            } => self.tell_host(link, |member_id| PeerEvent::MapLoaded {
                member_id,
                epoch,
                map,
                weapons,
                classes,
            }),
            R::BootstrapApplied {
                epoch,
                bootstrap_id,
                snapshot_seq,
                connection,
            } => self.tell_host(link, |member_id| PeerEvent::BootstrapApplied {
                member_id,
                epoch,
                bootstrap_id,
                snapshot_seq,
                connection,
            }),
            R::VoteToSkip => self.tell_host(link, |member_id| PeerEvent::VoteToSkip { member_id }),
            R::EnterMatch {
                member_id,
                epoch,
                bootstrap_id,
                connection_id,
                client_id,
            } => self.tell_member(
                link,
                member_id,
                PeerEvent::EnterMatch {
                    member_id,
                    epoch,
                    bootstrap_id,
                    connection_id,
                    client_id,
                },
            ),
            R::AdmissionFailed {
                member_id,
                epoch,
                reason,
            } => self.tell_member(link, member_id, admission_refusal(member_id, epoch, reason)),
        }
    }

    fn open(
        &mut self,
        link: Link,
        name: String,
        map: String,
        mode: String,
        max_players: u8,
        requires: ContentFlags,
    ) -> Answer {
        if self.seated_at.contains_key(&link) {
            return refuse(ServiceError::AlreadyHosting);
        }
        if self.tables.len() >= ROOM_LIMIT {
            return refuse(ServiceError::AdvertCapacity);
        }
        let room_id = AdvertId((self.fresh_id)());
        let host = MemberId((self.fresh_id)());
        let view = RoomView {
            room_id,
            name,
            host,
            members: vec![host],
            member_names: HashMap::from([(host, self.name_of(link))]),
            map,
            mode,
            joinable: true,
            revision: 1,
            epoch: 0,
            phase: RoomPhase::Gathering,
            max_players,
            requires,
        };
        self.tables.insert(
            room_id,
            Table {
                view: view.clone(),
                host_link: link,
                member_by_link: HashMap::from([(link, host)]),
                link_by_member: HashMap::from([(host, link)]),
            },
        );
        self.seated_at.insert(link, room_id);
        self.changed();
        let outbox = vec![(link, ControlFrame::RoomView(view.clone()))];
        (ResponseBody::RoomCreated { member_id: host, view }, outbox)
    }

    fn join(&mut self, link: Link, room_id: AdvertId, have: ContentFlags) -> Answer {
        if let Some(&current) = self.seated_at.get(&link) {
            if current != room_id {
                return refuse(ServiceError::AlreadyHosting);
            }
            // Asking again for the room you are in: the same seat, nothing new to tell.
            return match self.seat(link) {
                Ok((_, member_id)) => (
                    ResponseBody::RoomJoined {
                        member_id,
                        view: self.tables[&room_id].view.clone(),
                    },
                    Vec::new(),
                ),
                Err(error) => refuse(error),
            };
        }
        let name = self.name_of(link);
        let Some(table) = self.tables.get(&room_id) else {
            return refuse(ServiceError::UnknownAdvert);
        };
        let view = &table.view;
        if !view.advert().allows_join_request() {
            return refuse(ServiceError::Locked);
        }
        if !view.requires.missing_from(have).is_empty() {
            return refuse(ServiceError::MissingContent {
                requires: view.requires,
                have,
            });
        }
        if view.members.len() >= usize::from(view.max_players) {
            return refuse(ServiceError::Full);
        }
        let member_id = MemberId((self.fresh_id)());
        let Some(table) = self.tables.get_mut(&room_id) else {
            return refuse(ServiceError::UnknownAdvert);
        };
        table.member_by_link.insert(link, member_id);
        table.link_by_member.insert(member_id, link);
        table.view.members.push(member_id);
        table.view.member_names.insert(member_id, name);
        table.touch();
        let view = table.view.clone();
        let outbox = table.view_for_all();
        self.seated_at.insert(link, room_id);
        self.changed();
        (ResponseBody::RoomJoined { member_id, view }, outbox)
    }

    fn leave(&mut self, link: Link) -> Answer {
        let Some(&room_id) = self.seated_at.get(&link) else {
            return (ResponseBody::RoomLeft, Vec::new());
        };
        let hosting = self
            .tables
            .get(&room_id)
            .is_some_and(|table| table.host_link == link);
        let outbox = if hosting {
            self.close(room_id, SessionCloseReason::HostLeft)
        } else {
            self.unseat(link)
        };
        (ResponseBody::RoomLeft, outbox)
    }

    /// `SetOptions` (with `joinable`) or `StartMatch` (without). Once a match is on,
    /// the map and mode are fixed.
    fn configure(
        &mut self,
        link: Link,
        map: String,
        mode: String,
        joinable: Option<bool>,
    ) -> Answer {
        let table = match self.hosted_table(link) {
            Ok(table) => table,
            Err(error) => return refuse(error),
        };
        let playing = table.view.phase.in_match();
        if playing && (table.view.map != map || table.view.mode != mode) {
            return refuse(ServiceError::InvalidAdvert);
        }
        table.view.map = map;
        table.view.mode = mode;
        match joinable {
            Some(joinable) => table.view.joinable = joinable,
            // Starting a match that is already on changes nothing.
            None if playing => {
                let view = table.view.clone();
                return (ResponseBody::RoomUpdated { view }, Vec::new());
            }
            None => {
                table.view.epoch = table.view.epoch.wrapping_add(1).max(1);
                table.view.phase = RoomPhase::Loading;
            }
        }
        table.touch();
        let view = table.view.clone();
        let outbox = table.view_for_all();
        self.changed();
        (ResponseBody::RoomUpdated { view }, outbox)
    }

    /// `EndMatch` (no event: back to gathering) or `HostWorldReady` (with the event
    /// every member gets: the match is running). Stale or mismatched requests are
    /// acknowledged and ignored.
    fn phase_change(
        &mut self,
        link: Link,
        room_id: AdvertId,
        epoch: u32,
        ready: Option<PeerEvent>,
    ) -> Answer {
        let table = match self.hosted_table(link) {
            Ok(table) => table,
            Err(error) => return refuse(error),
        };
        if !table.running(room_id, epoch) {
            return (ResponseBody::Ack, Vec::new());
        }
        table.view.phase = if ready.is_some() {
            RoomPhase::Running
        } else {
            RoomPhase::Gathering
        };
        table.touch();
        let mut outbox = table.view_for_all();
        if let Some(event) = &ready {
            outbox.extend(table.event_for_all(event));
        }
        self.changed();
        (ResponseBody::Ack, outbox)
    }

    /// A member's admission news, stamped with its member id, for the host only.
    fn tell_host(&self, link: Link, event: impl FnOnce(MemberId) -> PeerEvent) -> Answer {
        let (room_id, member_id) = match self.seat(link) {
            Ok(seat) => seat,
            Err(error) => return refuse(error),
        };
        let Some(table) = self.tables.get(&room_id) else {
            return refuse(ServiceError::UnknownAdvert);
        };
        let frame = ControlFrame::PeerEvent(event(member_id));
        (ResponseBody::Ack, vec![(table.host_link, frame)])
    }

    /// The host's admission news for one of its members.
    fn tell_member(&self, link: Link, member_id: MemberId, event: PeerEvent) -> Answer {
        let room_id = match self.hosted(link) {
            Ok(room_id) => room_id,
            Err(error) => return refuse(error),
        };
        let Some(table) = self.tables.get(&room_id) else {
            return refuse(ServiceError::UnknownAdvert);
        };
        match table.link_by_member.get(&member_id) {
            Some(&to) => (ResponseBody::Ack, vec![(to, ControlFrame::PeerEvent(event))]),
            None => refuse(ServiceError::NotMember),
        }
    }

    /// Up to 128 rooms, by id.
    fn listing(&self) -> ResponseBody {
        let mut adverts: Vec<Advert> = self.tables.values().map(|table| table.view.advert()).collect();
        adverts.sort_unstable_by_key(|advert| advert.id);
        adverts.truncate(MAX_LIST_ADVERTS);
        ResponseBody::RoomList {
            generation: self.generation,
            adverts,
        }
    }

    /// Removes a room; every member hears why.
    fn close(&mut self, room_id: AdvertId, reason: SessionCloseReason) -> Outbox {
        let Some(table) = self.tables.remove(&room_id) else {
            return Vec::new();
        };
        self.changed();
        let notice = ControlFrame::Closed {
            room_id,
            epoch: table.view.epoch,
            reason,
        };
        table
            .member_by_link
            .into_keys()
            .map(|link| {
                self.seated_at.remove(&link);
                (link, notice.clone())
            })
            .collect()
    }

    /// Takes a non-host member out of its room; the rest see the new view.
    fn unseat(&mut self, link: Link) -> Outbox {
        let Some(room_id) = self.seated_at.remove(&link) else {
            return Vec::new();
        };
        let Some(table) = self.tables.get_mut(&room_id) else {
            return Vec::new();
        };
        let Some(member_id) = table.member_by_link.remove(&link) else {
            return Vec::new();
        };
        table.link_by_member.remove(&member_id);
        table.view.members.retain(|&kept| kept != member_id);
        table.view.member_names.remove(&member_id);
        table.touch();
        let outbox = table.view_for_all();
        self.changed();
        outbox
    }

    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn name_of(&self, link: Link) -> String {
        self.names
            .get(&link)
            .cloned()
            .unwrap_or_else(|| FALLBACK_NAME.to_owned())
    }
}

fn admission_refusal(member_id: MemberId, epoch: u32, reason: AdmissionFailure) -> PeerEvent {
    PeerEvent::AdmissionFailed {
        member_id,
        epoch,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: Link = 1;
    const GUEST: Link = 2;
    const THIRD: Link = 3;

    /// The n-th id the test lobby hands out (from 1), so tests can name them.
    fn nth(n: u64) -> [u8; 16] {
        let mut raw = [0; 16];
        raw[8..].copy_from_slice(&n.to_be_bytes());
        raw
    }

    fn lobby() -> Lobby {
        let mut handed_out = 0;
        let mut lobby = Lobby::new(move || {
            handed_out += 1;
            nth(handed_out)
        });
        lobby.arrive(HOST, "Ann".into());
        lobby.arrive(GUEST, "Bo".into());
        lobby.arrive(THIRD, "Cy".into());
        lobby
    }

    fn create(lobby: &mut Lobby, link: Link) -> (AdvertId, MemberId) {
        let (answer, _) = lobby.answer(
            link,
            RequestBody::CreateRoom {
                name: "room".into(),
                map: "mp_raid".into(),
                mode: "tdm".into(),
                max_players: 3,
                requires: ContentFlags(0),
            },
        );
        match answer {
            ResponseBody::RoomCreated { member_id, view } => (view.room_id, member_id),
            other => panic!("create refused: {other:?}"),
        }
    }

    fn join(lobby: &mut Lobby, link: Link, room_id: AdvertId) -> MemberId {
        let (answer, _) = lobby.answer(
            link,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0),
            },
        );
        match answer {
            ResponseBody::RoomJoined { member_id, .. } => member_id,
            other => panic!("join refused: {other:?}"),
        }
    }

    fn sorted(mut outbox: Outbox) -> Outbox {
        outbox.sort_by_key(|(link, _)| *link);
        outbox
    }

    fn generation(lobby: &Lobby) -> u64 {
        match lobby.listing() {
            ResponseBody::RoomList { generation, .. } => generation,
            other => panic!("{other:?}"),
        }
    }

    fn view_of(lobby: &Lobby, room_id: AdvertId) -> RoomView {
        lobby.tables[&room_id].view.clone()
    }

    fn start(lobby: &mut Lobby) -> Answer {
        lobby.answer(
            HOST,
            RequestBody::StartMatch {
                map: "mp_raid".into(),
                mode: "tdm".into(),
            },
        )
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(display_name("  Ann \t"), "Ann");
        assert_eq!(display_name("A\u{7}n\nn"), "Ann");
        assert_eq!(display_name("abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnop");
        assert_eq!(display_name(" \r\n "), "Player");
        assert_eq!(display_name(""), "Player");
    }

    #[test]
    fn a_new_room_has_only_its_host() {
        let mut lobby = lobby();
        let (answer, outbox) = lobby.answer(
            HOST,
            RequestBody::CreateRoom {
                name: "Ann's room".into(),
                map: "mp_raid".into(),
                mode: "tdm".into(),
                max_players: 12,
                requires: ContentFlags(2),
            },
        );
        let ResponseBody::RoomCreated { member_id, view } = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(view.room_id, AdvertId(nth(1)));
        assert_eq!(member_id, MemberId(nth(2)));
        assert_eq!(view.host, member_id);
        assert_eq!(view.members, vec![member_id]);
        assert_eq!(view.member_names[&member_id], "Ann");
        assert!(view.joinable);
        assert_eq!((view.revision, view.epoch), (1, 0));
        assert_eq!(view.phase, RoomPhase::Gathering);
        assert_eq!((view.max_players, view.requires), (12, ContentFlags(2)));
        assert_eq!(outbox, vec![(HOST, ControlFrame::RoomView(view.clone()))]);
        assert_eq!(lobby.seat(HOST), Ok((view.room_id, member_id)));
        assert_eq!(generation(&lobby), 1);
    }

    #[test]
    fn one_room_per_connection() {
        let mut lobby = lobby();
        let (room_id, _) = create(&mut lobby, HOST);
        let (again, _) = lobby.answer(
            HOST,
            RequestBody::CreateRoom {
                name: "x".into(),
                map: "m".into(),
                mode: "d".into(),
                max_players: 2,
                requires: ContentFlags(0),
            },
        );
        assert_eq!(again, ResponseBody::Error(ServiceError::AlreadyHosting));
        let (other, _) = create(&mut lobby, GUEST);
        let (cross, _) = lobby.answer(
            GUEST,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0),
            },
        );
        assert_ne!(other, room_id);
        assert_eq!(cross, ResponseBody::Error(ServiceError::AlreadyHosting));
    }

    #[test]
    fn room_limit() {
        let mut lobby = lobby();
        for link in 100..100 + ROOM_LIMIT as Link {
            create(&mut lobby, link);
        }
        let (answer, outbox) = lobby.answer(
            HOST,
            RequestBody::CreateRoom {
                name: "late".into(),
                map: "m".into(),
                mode: "d".into(),
                max_players: 2,
                requires: ContentFlags(0),
            },
        );
        assert_eq!(answer, ResponseBody::Error(ServiceError::AdvertCapacity));
        assert!(outbox.is_empty());
    }

    #[test]
    fn joining_adds_a_member_everyone_sees() {
        let mut lobby = lobby();
        let (room_id, host) = create(&mut lobby, HOST);
        let (answer, outbox) = lobby.answer(
            GUEST,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0),
            },
        );
        let ResponseBody::RoomJoined { member_id, view } = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(view.members, vec![host, member_id]);
        assert_eq!(view.member_names[&member_id], "Bo");
        assert_eq!(view.revision, 2);
        assert_eq!(generation(&lobby), 2);
        let frame = ControlFrame::RoomView(view.clone());
        assert_eq!(sorted(outbox), vec![(HOST, frame.clone()), (GUEST, frame)]);

        // Asking again changes nothing and tells nobody.
        let (again, outbox) = lobby.answer(
            GUEST,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0),
            },
        );
        assert_eq!(again, ResponseBody::RoomJoined { member_id, view });
        assert!(outbox.is_empty());
        assert_eq!(generation(&lobby), 2);
    }

    #[test]
    fn join_refusals() {
        let mut lobby = lobby();
        let (unknown, _) = lobby.answer(
            GUEST,
            RequestBody::JoinRoom {
                room_id: AdvertId([9; 16]),
                have: ContentFlags(0),
            },
        );
        assert_eq!(unknown, ResponseBody::Error(ServiceError::UnknownAdvert));

        let (_, outbox) = lobby.answer(
            HOST,
            RequestBody::CreateRoom {
                name: "dlc".into(),
                map: "m".into(),
                mode: "d".into(),
                max_players: 2,
                requires: ContentFlags(0b101),
            },
        );
        let room_id = view_from(&outbox).room_id;
        let (missing, _) = lobby.answer(
            GUEST,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0b001),
            },
        );
        assert_eq!(
            missing,
            ResponseBody::Error(ServiceError::MissingContent {
                requires: ContentFlags(0b101),
                have: ContentFlags(0b001),
            })
        );
        let (joined, _) = lobby.answer(
            GUEST,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0b111),
            },
        );
        assert!(matches!(joined, ResponseBody::RoomJoined { .. }));
        let (full, _) = lobby.answer(
            THIRD,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0b111),
            },
        );
        assert_eq!(full, ResponseBody::Error(ServiceError::Full));

        let (_, _) = lobby.answer(
            HOST,
            RequestBody::SetOptions {
                map: "m".into(),
                mode: "d".into(),
                joinable: false,
            },
        );
        let (_, _) = lobby.answer(GUEST, RequestBody::LeaveRoom);
        let (locked, _) = lobby.answer(
            THIRD,
            RequestBody::JoinRoom {
                room_id,
                have: ContentFlags(0b111),
            },
        );
        assert_eq!(locked, ResponseBody::Error(ServiceError::Locked));
    }

    fn view_from(outbox: &Outbox) -> RoomView {
        match &outbox[0].1 {
            ControlFrame::RoomView(view) => view.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_member_leaving_updates_the_rest() {
        let mut lobby = lobby();
        let (room_id, host) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, room_id);
        let third = join(&mut lobby, THIRD, room_id);
        let (answer, outbox) = lobby.answer(GUEST, RequestBody::LeaveRoom);
        assert_eq!(answer, ResponseBody::RoomLeft);
        let view = view_of(&lobby, room_id);
        assert_eq!(view.members, vec![host, third]);
        assert_eq!(view.member_names.len(), 2);
        assert_eq!(view.revision, 4);
        let frame = ControlFrame::RoomView(view);
        assert_eq!(sorted(outbox), vec![(HOST, frame.clone()), (THIRD, frame)]);
        assert_eq!(lobby.seat(GUEST), Err(ServiceError::NotMember));
        assert_eq!(generation(&lobby), 4);

        let (idle, outbox) = lobby.answer(GUEST, RequestBody::LeaveRoom);
        assert_eq!(idle, ResponseBody::RoomLeft);
        assert!(outbox.is_empty());
    }

    #[test]
    fn the_host_leaving_closes_the_room() {
        for how in 0..3 {
            let mut lobby = lobby();
            let (room_id, _) = create(&mut lobby, HOST);
            join(&mut lobby, GUEST, room_id);
            let outbox = match how {
                0 => lobby.answer(HOST, RequestBody::LeaveRoom).1,
                1 => {
                    let (answer, outbox) = lobby.answer(HOST, RequestBody::CloseRoom);
                    assert_eq!(answer, ResponseBody::Ack);
                    outbox
                }
                _ => lobby.depart(HOST),
            };
            let notice = ControlFrame::Closed {
                room_id,
                epoch: 0,
                reason: SessionCloseReason::HostLeft,
            };
            assert_eq!(sorted(outbox), vec![(HOST, notice.clone()), (GUEST, notice)]);
            assert_eq!(lobby.seat(GUEST), Err(ServiceError::NotMember));
            assert_eq!(generation(&lobby), 3);
            let ResponseBody::RoomList { adverts, .. } = lobby.listing() else {
                unreachable!()
            };
            assert!(adverts.is_empty());
            // Both are free to start over.
            create(&mut lobby, GUEST);
        }
    }

    #[test]
    fn a_member_disconnecting_leaves_the_room() {
        let mut lobby = lobby();
        let (room_id, host) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, room_id);
        let outbox = lobby.depart(GUEST);
        let view = view_of(&lobby, room_id);
        assert_eq!(view.members, vec![host]);
        assert_eq!(outbox, vec![(HOST, ControlFrame::RoomView(view))]);
        assert!(lobby.depart(GUEST).is_empty());
        assert!(lobby.depart(99).is_empty());
    }

    #[test]
    fn only_the_host_changes_options() {
        let mut lobby = lobby();
        let options = || RequestBody::SetOptions {
            map: "mp_slums".into(),
            mode: "dom".into(),
            joinable: false,
        };
        assert_eq!(
            lobby.answer(HOST, options()).0,
            ResponseBody::Error(ServiceError::NotMember)
        );
        let (room_id, _) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, room_id);
        assert_eq!(
            lobby.answer(GUEST, options()).0,
            ResponseBody::Error(ServiceError::NotHost)
        );
        let (answer, outbox) = lobby.answer(HOST, options());
        let view = view_of(&lobby, room_id);
        assert_eq!((view.map.as_str(), view.mode.as_str()), ("mp_slums", "dom"));
        assert!(!view.joinable);
        assert_eq!(view.revision, 3);
        assert_eq!(answer, ResponseBody::RoomUpdated { view: view.clone() });
        assert_eq!(outbox.len(), 2);
        assert_eq!(generation(&lobby), 3);
    }

    #[test]
    fn starting_a_match() {
        let mut lobby = lobby();
        let (room_id, _) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, room_id);
        let (answer, outbox) = start(&mut lobby);
        let view = view_of(&lobby, room_id);
        assert_eq!((view.epoch, view.phase, view.revision), (1, RoomPhase::Loading, 3));
        assert_eq!(answer, ResponseBody::RoomUpdated { view: view.clone() });
        assert_eq!(outbox.len(), 2);

        // Again with the same settings: answered, nothing changes.
        let (answer, outbox) = start(&mut lobby);
        assert_eq!(answer, ResponseBody::RoomUpdated { view: view.clone() });
        assert!(outbox.is_empty());
        assert_eq!(generation(&lobby), 3);

        // The map and mode are fixed for the match, in either request.
        let moved = lobby.answer(
            HOST,
            RequestBody::StartMatch {
                map: "mp_nuketown_2020".into(),
                mode: "tdm".into(),
            },
        );
        assert_eq!(moved.0, ResponseBody::Error(ServiceError::InvalidAdvert));
        let moved = lobby.answer(
            HOST,
            RequestBody::SetOptions {
                map: "mp_raid".into(),
                mode: "sd".into(),
                joinable: true,
            },
        );
        assert_eq!(moved.0, ResponseBody::Error(ServiceError::InvalidAdvert));
        // Locking the room mid-match is fine.
        let locked = lobby.answer(
            HOST,
            RequestBody::SetOptions {
                map: "mp_raid".into(),
                mode: "tdm".into(),
                joinable: false,
            },
        );
        assert!(matches!(locked.0, ResponseBody::RoomUpdated { .. }));
        assert_eq!(view_of(&lobby, room_id).revision, 4);
    }

    #[test]
    fn world_ready_then_end() {
        let mut lobby = lobby();
        let (room_id, _) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, room_id);
        let ready = |room_id, epoch| RequestBody::HostWorldReady {
            room_id,
            epoch,
            map: 11,
            weapons: 22,
            classes: 33,
        };
        // Before any match, and for the wrong match: acknowledged, ignored.
        assert_eq!(lobby.answer(HOST, ready(room_id, 0)), (ResponseBody::Ack, Vec::new()));
        start(&mut lobby);
        assert_eq!(lobby.answer(HOST, ready(room_id, 2)), (ResponseBody::Ack, Vec::new()));
        assert_eq!(
            lobby.answer(HOST, ready(AdvertId([7; 16]), 1)),
            (ResponseBody::Ack, Vec::new())
        );
        assert_eq!(
            lobby.answer(GUEST, ready(room_id, 1)).0,
            ResponseBody::Error(ServiceError::NotHost)
        );

        let (answer, outbox) = lobby.answer(HOST, ready(room_id, 1));
        assert_eq!(answer, ResponseBody::Ack);
        let view = view_of(&lobby, room_id);
        assert_eq!((view.phase, view.revision), (RoomPhase::Running, 4));
        let event = ControlFrame::PeerEvent(PeerEvent::HostWorldReady {
            epoch: 1,
            map: 11,
            weapons: 22,
            classes: 33,
        });
        // The views go out before the event.
        assert!(outbox[..2]
            .iter()
            .all(|(_, frame)| matches!(frame, ControlFrame::RoomView(_))));
        let mut events: Vec<_> = outbox[2..].to_vec();
        events.sort_by_key(|(link, _)| *link);
        assert_eq!(events, vec![(HOST, event.clone()), (GUEST, event)]);

        let end = |epoch| RequestBody::EndMatch { room_id, epoch };
        assert_eq!(lobby.answer(HOST, end(5)), (ResponseBody::Ack, Vec::new()));
        let (answer, outbox) = lobby.answer(HOST, end(1));
        assert_eq!(answer, ResponseBody::Ack);
        assert_eq!(outbox.len(), 2);
        let view = view_of(&lobby, room_id);
        assert_eq!((view.phase, view.epoch, view.revision), (RoomPhase::Gathering, 1, 5));
        // Already ended.
        assert_eq!(lobby.answer(HOST, end(1)), (ResponseBody::Ack, Vec::new()));
        // The next match is number 2.
        start(&mut lobby);
        assert_eq!(view_of(&lobby, room_id).epoch, 2);
    }

    #[test]
    fn member_news_goes_to_the_host() {
        let mut lobby = lobby();
        let (room_id, _) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, room_id);
        let (answer, outbox) = lobby.answer(
            GUEST,
            RequestBody::MapLoaded {
                epoch: 1,
                map: 4,
                weapons: 5,
                classes: 6,
            },
        );
        assert_eq!(answer, ResponseBody::Ack);
        assert_eq!(
            outbox,
            vec![(
                HOST,
                ControlFrame::PeerEvent(PeerEvent::MapLoaded {
                    member_id: guest,
                    epoch: 1,
                    map: 4,
                    weapons: 5,
                    classes: 6,
                })
            )]
        );
        let (_, outbox) = lobby.answer(
            GUEST,
            RequestBody::BootstrapApplied {
                epoch: 1,
                bootstrap_id: 7,
                snapshot_seq: 8,
                connection: 9,
            },
        );
        assert_eq!(
            outbox,
            vec![(
                HOST,
                ControlFrame::PeerEvent(PeerEvent::BootstrapApplied {
                    member_id: guest,
                    epoch: 1,
                    bootstrap_id: 7,
                    snapshot_seq: 8,
                    connection: 9,
                })
            )]
        );
        let (_, outbox) = lobby.answer(GUEST, RequestBody::VoteToSkip);
        assert_eq!(
            outbox,
            vec![(HOST, ControlFrame::PeerEvent(PeerEvent::VoteToSkip { member_id: guest }))]
        );
        assert_eq!(
            lobby.answer(THIRD, RequestBody::VoteToSkip),
            (ResponseBody::Error(ServiceError::NotMember), Vec::new())
        );
    }

    #[test]
    fn host_news_goes_to_one_member() {
        let mut lobby = lobby();
        let (room_id, _) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, room_id);
        join(&mut lobby, THIRD, room_id);
        let enter = |member_id| RequestBody::EnterMatch {
            member_id,
            epoch: 1,
            bootstrap_id: 2,
            connection_id: 3,
            client_id: 4,
        };
        let (answer, outbox) = lobby.answer(HOST, enter(guest));
        assert_eq!(answer, ResponseBody::Ack);
        assert_eq!(
            outbox,
            vec![(
                GUEST,
                ControlFrame::PeerEvent(PeerEvent::EnterMatch {
                    member_id: guest,
                    epoch: 1,
                    bootstrap_id: 2,
                    connection_id: 3,
                    client_id: 4,
                })
            )]
        );
        let (_, outbox) = lobby.answer(
            HOST,
            RequestBody::AdmissionFailed {
                member_id: guest,
                epoch: 1,
                reason: AdmissionFailure::Deadline,
            },
        );
        assert_eq!(
            outbox,
            vec![(
                GUEST,
                ControlFrame::PeerEvent(PeerEvent::AdmissionFailed {
                    member_id: guest,
                    epoch: 1,
                    reason: AdmissionFailure::Deadline,
                })
            )]
        );
        assert_eq!(
            lobby.answer(HOST, enter(MemberId([0xee; 16]))).0,
            ResponseBody::Error(ServiceError::NotMember)
        );
        assert_eq!(
            lobby.answer(GUEST, enter(guest)).0,
            ResponseBody::Error(ServiceError::NotHost)
        );
    }

    #[test]
    fn packets_route_through_the_room() {
        let mut lobby = lobby();
        let (room_id, _) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, room_id);
        let up = RelayDatagram::ClientToHost(b"up");
        assert_eq!(
            lobby.route(GUEST, &up),
            Ok(Hop {
                to: HOST,
                to_host: true,
                packet: RelayDatagram::ServiceToHost {
                    member_id: guest,
                    payload: b"up",
                },
            })
        );
        let down = RelayDatagram::HostToMember {
            member_id: guest,
            payload: b"down",
        };
        assert_eq!(
            lobby.route(HOST, &down),
            Ok(Hop {
                to: GUEST,
                to_host: false,
                packet: RelayDatagram::ServiceToMember(b"down"),
            })
        );
        assert_eq!(lobby.route(THIRD, &up), Err(ServiceError::NotMember));
        assert_eq!(lobby.route(GUEST, &down), Err(ServiceError::NotHost));
        let stranger = RelayDatagram::HostToMember {
            member_id: MemberId([0xee; 16]),
            payload: b"",
        };
        assert_eq!(lobby.route(HOST, &stranger), Err(ServiceError::NotMember));
        assert_eq!(
            lobby.route(HOST, &RelayDatagram::ServiceToMember(b"")),
            Err(ServiceError::Malformed)
        );
    }

    #[test]
    fn the_list_is_sorted_and_capped() {
        let mut lobby = lobby();
        for link in 10..10 + MAX_LIST_ADVERTS as Link + 5 {
            create(&mut lobby, link);
        }
        let ResponseBody::RoomList {
            generation,
            adverts,
        } = lobby.listing()
        else {
            unreachable!()
        };
        assert_eq!(generation, MAX_LIST_ADVERTS as u64 + 5);
        assert_eq!(adverts.len(), MAX_LIST_ADVERTS);
        assert!(adverts.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_eq!(adverts[0].players, 1);
    }
}
