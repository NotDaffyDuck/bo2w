//! The lobby's bookkeeping: who is connected, which rooms exist and who sits where.
//! Nothing here touches a socket. Every change hands back the frames it owes to
//! other connections, and the QUIC side delivers them.

use std::collections::HashMap;

use master_protocol::*;

/// The number the accept loop gives each connection, starting at 1.
pub type Link = u64;

/// Frames owed to connections, in the order they should go out.
pub type Outbox = Vec<(Link, ControlEnvelope)>;

/// The most rooms open at once; one more is refused.
pub const ROOM_LIMIT: usize = 10_000;
/// Longest display name kept, in characters.
const NAME_CHARS: usize = 16;
/// The name shown for a player who sent none.
const FALLBACK_NAME: &str = "Player";
/// A new room's first revision.
const FIRST_REVISION: u64 = 1;

/// A player's chosen name as other players will see it: trimmed, control
/// characters dropped, at most 16 characters, never empty.
pub fn shown_label(raw: &str) -> String {
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
    view: LobbyOverview,
    host_link: Link,
    member_by_link: HashMap<Link, RoomOccupantId>,
    link_by_member: HashMap<RoomOccupantId, Link>,
}

impl Table {
    /// Records a change the members must see.
    fn bump_revision(&mut self) {
        self.view.revision = self.view.revision.wrapping_add(1).max(1);
    }

    fn view_for_all(&self) -> Outbox {
        let frame = ControlEnvelope::LobbyOverview(self.view.clone());
        self.member_by_link.keys().map(|&link| (link, frame.clone())).collect()
    }

    fn event_for_all(&self, event: &RoomMemberNotice) -> Outbox {
        let frame = ControlEnvelope::RoomMemberNotice(event.clone());
        self.member_by_link.keys().map(|&link| (link, frame.clone())).collect()
    }

    /// The host's connection is past match start for this exact match.
    fn running(&self, lobby_number: ListingId, match_epoch: u32) -> bool {
        lobby_number == self.view.lobby_number
            && match_epoch != 0
            && match_epoch == self.view.epoch
            && self.view.phase.is_playing()
    }
}

/// Where a relayed packet goes next and what it says when it gets there.
#[derive(Debug, PartialEq, Eq)]
pub struct Hop<'a> {
    pub to: Link,
    /// True when `to` is the room's host (a missing host connection is `NotRoomOwner`).
    pub to_host: bool,
    pub packet: ForwardedPacket<'a>,
}

/// Every room and every connected player.
pub struct Lobby {
    player_names: HashMap<Link, String>,
    tables: HashMap<ListingId, Table>,
    seated_at: HashMap<Link, ListingId>,
    /// Goes up on any change to any room; the room list reports it.
    list_stamp: u64,
    fresh_id: Box<dyn FnMut() -> [u8; 16] + Send>,
}

type Answer = (ReplyPayload, Outbox);

fn refuse(error: DirectoryServiceFault) -> Answer {
    (ReplyPayload::Error(error), Outbox::new())
}

impl Lobby {
    /// `fresh_id` supplies the random room and member ids.
    pub fn new(fresh_id: impl FnMut() -> [u8; 16] + Send + 'static) -> Self {
        Self {
            player_names: HashMap::new(),
            tables: HashMap::new(),
            seated_at: HashMap::new(),
            list_stamp: 0,
            fresh_id: Box::new(fresh_id),
        }
    }

    /// A connection said hello with this (already cleaned) name.
    pub fn arrive(&mut self, link: Link, label: String) {
        self.player_names.insert(link, label);
    }

    /// A connection is gone. A host takes its room with it; a member just leaves.
    pub fn depart(&mut self, link: Link) -> Outbox {
        self.player_names.remove(&link);
        let Some(&lobby_number) = self.seated_at.get(&link) else {
            return Outbox::new();
        };
        match self.tables.get(&lobby_number) {
            None => {
                self.seated_at.remove(&link);
                Outbox::new()
            }
            Some(table) if table.host_link == link => {
                self.close(lobby_number, RoomShutdownCause::HostLeft)
            }
            Some(_) => self.unseat(link),
        }
    }

    /// The room and member id of a seated connection.
    pub fn seat(&self, link: Link) -> Result<(ListingId, RoomOccupantId), DirectoryServiceFault> {
        let lobby_number = *self.seated_at.get(&link).ok_or(DirectoryServiceFault::NotRoomOccupant)?;
        let table = self.tables.get(&lobby_number).ok_or(DirectoryServiceFault::NoSuchListing)?;
        let member = *table.member_by_link.get(&link).ok_or(DirectoryServiceFault::NotRoomOccupant)?;
        Ok((lobby_number, member))
    }

    /// The room this connection hosts.
    fn hosted(&self, link: Link) -> Result<ListingId, DirectoryServiceFault> {
        let lobby_number = *self.seated_at.get(&link).ok_or(DirectoryServiceFault::NotRoomOccupant)?;
        let table = self.tables.get(&lobby_number).ok_or(DirectoryServiceFault::NoSuchListing)?;
        if table.host_link == link {
            Ok(lobby_number)
        } else {
            Err(DirectoryServiceFault::NotRoomOwner)
        }
    }

    fn hosted_table(&mut self, link: Link) -> Result<&mut Table, DirectoryServiceFault> {
        let lobby_number = self.hosted(link)?;
        self.tables.get_mut(&lobby_number).ok_or(DirectoryServiceFault::NoSuchListing)
    }

    /// Where a packet from `link` must go: a member's packet to its host, a host's
    /// packet to the member it names. Packets already marked as from the service
    /// are refused.
    pub fn route<'a>(
        &self,
        link: Link,
        packet: &ForwardedPacket<'a>,
    ) -> Result<Hop<'a>, DirectoryServiceFault> {
        match *packet {
            ForwardedPacket::ReceiverToHost(body) => {
                let (lobby_number, seat_id) = self.seat(link)?;
                let table = self.tables.get(&lobby_number).ok_or(DirectoryServiceFault::NoSuchListing)?;
                Ok(Hop {
                    to: table.host_link,
                    to_host: true,
                    packet: ForwardedPacket::DirectoryToHost { member_id: seat_id, payload: body },
                })
            }
            ForwardedPacket::HostToParticipant { payload, member_id: to_member } => {
                let lobby_number = self.hosted(link)?;
                let table = self.tables.get(&lobby_number).ok_or(DirectoryServiceFault::NoSuchListing)?;
                let to = *table
                    .link_by_member
                    .get(&to_member)
                    .ok_or(DirectoryServiceFault::NotRoomOccupant)?;
                Ok(Hop {
                    to,
                    to_host: false,
                    packet: ForwardedPacket::DirectoryToParticipant(payload),
                })
            }
            ForwardedPacket::DirectoryToHost { .. } | ForwardedPacket::DirectoryToParticipant(_) => {
                Err(DirectoryServiceFault::GarbledRequest)
            }
        }
    }

    /// Carries out one request. The answer goes to `link` after the outbox.
    pub fn answer(&mut self, link: Link, request: QueryPayload) -> Answer {
        use QueryPayload as R;
        match request {
            R::Status => (ReplyPayload::Status(HealthReply::CURRENT), Outbox::new()),
            R::OpenLobby {
                name,
                map,
                mode,
                seat_capacity,
                requires,
            } => self.open(link, name, map, mode, seat_capacity, requires),
            R::JoinRoom { lobby_number, have } => self.join(link, lobby_number, have),
            R::LeaveRoom => self.leave(link),
            R::ConfigureLobby {
                map,
                mode,
                open_to_join,
            } => self.configure(link, map, mode, Some(open_to_join)),
            R::BeginGame { map, mode } => self.configure(link, map, mode, None),
            R::EndMatch { lobby_number, epoch } => self.phase_change(link, lobby_number, epoch, None),
            R::CloseRoom => match self.hosted(link) {
                Ok(lobby_number) => (
                    ReplyPayload::Ack,
                    self.close(lobby_number, RoomShutdownCause::HostLeft),
                ),
                Err(error) => refuse(error),
            },
            R::ListRooms => (self.listing(), Outbox::new()),
            R::HostMapLoaded {
                lobby_number,
                epoch,
                map,
                weapons,
                classes,
            } => {
                let ready = RoomMemberNotice::HostMapLoaded {
                    epoch,
                    map,
                    weapon_digest: weapons,
                    class_digest: classes,
                };
                self.phase_change(link, lobby_number, epoch, Some(ready))
            }
            R::MapLoaded {
                epoch,
                map,
                weapons,
                classes,
            } => self.tell_host(link, |member_id| RoomMemberNotice::MapLoaded {
                member_id,
                epoch,
                map,
                weapon_digest: weapons,
                class_digest: classes,
            }),
            R::TransferInstalledMsg {
                epoch,
                bootstrap_id: boot_id,
                frame_number_on_wire,
                connection,
            } => self.tell_host(link, |member_id| RoomMemberNotice::TransferInstalledMsg {
                member_id,
                epoch,
                transfer_id: boot_id,
                frame_number_on_wire,
                connection,
            }),
            R::BallotForSkip => self.tell_host(link, |member_id| RoomMemberNotice::BallotForSkip { member_id }),
            R::KickMember { member_id: target } => self.kick(link, target),
            R::JoinGame {
                member_id,
                epoch,
                bootstrap_id: boot_id,
                connection_id: conn_id,
                player_slot_no,
            } => self.tell_member(
                link,
                member_id,
                RoomMemberNotice::JoinGame {
                    member_id,
                    epoch,
                    transfer_id: boot_id,
                    link_id: conn_id,
                    player_slot_no,
                },
            ),
            R::EntryRefused {
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
        game_mode: String,
        seat_capacity: u8,
        requires: AssetCoverageBits,
    ) -> Answer {
        if self.seated_at.contains_key(&link) {
            return refuse(DirectoryServiceFault::HostSlotTaken);
        }
        if self.tables.len() >= ROOM_LIMIT {
            return refuse(DirectoryServiceFault::ListingsFull);
        }
        let lobby_number = ListingId((self.fresh_id)());
        let host = RoomOccupantId((self.fresh_id)());
        let view = LobbyOverview {
            lobby_number,
            name,
            host,
            members: vec![host],
            member_names: HashMap::from([(host, self.name_of(link))]),
            map,
            mode: game_mode,
            open_to_join: true,
            revision: FIRST_REVISION,
            epoch: 0,
            phase: LobbyStage::Gathering,
            seat_capacity,
            requires,
        };
        let shown = view.clone();
        self.tables.insert(
            lobby_number,
            Table {
                view: shown,
                host_link: link,
                member_by_link: HashMap::from([(link, host)]),
                link_by_member: HashMap::from([(host, link)]),
            },
        );
        self.seated_at.insert(link, lobby_number);
        self.changed();
        let outbox = vec![(link, ControlEnvelope::LobbyOverview(view.clone()))];
        (ReplyPayload::LobbyOpened { member_id: host, view }, outbox)
    }

    fn join(&mut self, link: Link, lobby_number: ListingId, have: AssetCoverageBits) -> Answer {
        if let Some(&current) = self.seated_at.get(&link) {
            if current != lobby_number {
                return refuse(DirectoryServiceFault::HostSlotTaken);
            }
            // Asking again for the room you are in: the same seat, nothing new to tell.
            return match self.seat(link) {
                Ok((_, member_id)) => (
                    ReplyPayload::LobbyEntered {
                        member_id,
                        view: self.tables[&lobby_number].view.clone(),
                    },
                    Outbox::new(),
                ),
                Err(error) => refuse(error),
            };
        }
        let name = self.name_of(link);
        let Some(table) = self.tables.get(&lobby_number) else {
            return refuse(DirectoryServiceFault::NoSuchListing);
        };
        let view = &table.view;
        if !view.advert().permits_join_ask() {
            return refuse(DirectoryServiceFault::Locked);
        }
        if !view.requires.lacking_in(have).is_empty() {
            return refuse(DirectoryServiceFault::AbsentResource {
                requires: view.requires,
                have,
            });
        }
        if view.members.len() >= usize::from(view.seat_capacity) {
            return refuse(DirectoryServiceFault::Full);
        }
        let newcomer = RoomOccupantId((self.fresh_id)());
        let Some(table) = self.tables.get_mut(&lobby_number) else {
            return refuse(DirectoryServiceFault::NoSuchListing);
        };
        table.member_by_link.insert(link, newcomer);
        table.link_by_member.insert(newcomer, link);
        table.view.members.push(newcomer);
        table.view.member_names.insert(newcomer, name);
        table.bump_revision();
        let view = table.view.clone();
        let outbox = table.view_for_all();
        self.seated_at.insert(link, lobby_number);
        self.changed();
        (ReplyPayload::LobbyEntered { member_id: newcomer, view }, outbox)
    }

    fn leave(&mut self, link: Link) -> Answer {
        let Some(&lobby_number) = self.seated_at.get(&link) else {
            return (ReplyPayload::RoomLeft, Outbox::new());
        };
        let hosting = self
            .tables
            .get(&lobby_number)
            .is_some_and(|table| table.host_link == link);
        let outbox = if hosting {
            self.close(lobby_number, RoomShutdownCause::HostLeft)
        } else {
            self.unseat(link)
        };
        (ReplyPayload::RoomLeft, outbox)
    }

    /// `ConfigureLobby` (with `open_to_join`) or `BeginGame` (without). Once a match is on,
    /// the map and mode are fixed.
    fn configure(
        &mut self,
        link: Link,
        map: String,
        game_mode: String,
        open_to_join: Option<bool>,
    ) -> Answer {
        let table = match self.hosted_table(link) {
            Ok(table) => table,
            Err(error) => return refuse(error),
        };
        let playing = table.view.phase.is_playing();
        if playing && (table.view.map != map || table.view.mode != game_mode) {
            return refuse(DirectoryServiceFault::ListingRejected);
        }
        table.view.map = map;
        table.view.mode = game_mode;
        match open_to_join {
            Some(open_to_join) => table.view.open_to_join = open_to_join,
            // Starting a match that is already on changes nothing.
            None if playing => {
                let view = table.view.clone();
                return (ReplyPayload::RoomUpdated { view }, Outbox::new());
            }
            None => {
                table.view.epoch = table.view.epoch.wrapping_add(1).max(1);
                table.view.phase = LobbyStage::Loading;
            }
        }
        table.bump_revision();
        let view = table.view.clone();
        let outbox = table.view_for_all();
        self.changed();
        (ReplyPayload::RoomUpdated { view }, outbox)
    }

    /// `EndMatch` (no event: back to gathering) or `HostMapLoaded` (with the event
    /// every member gets: the match is running). Stale or mismatched requests are
    /// acknowledged and ignored.
    fn phase_change(
        &mut self,
        link: Link,
        lobby_number: ListingId,
        epoch: u32,
        ready: Option<RoomMemberNotice>,
    ) -> Answer {
        let table = match self.hosted_table(link) {
            Ok(table) => table,
            Err(error) => return refuse(error),
        };
        if !table.running(lobby_number, epoch) {
            return (ReplyPayload::Ack, Outbox::new());
        }
        table.view.phase = if ready.is_some() {
            LobbyStage::Running
        } else {
            LobbyStage::Gathering
        };
        table.bump_revision();
        let mut outbox = table.view_for_all();
        if let Some(event) = &ready {
            outbox.extend(table.event_for_all(event));
        }
        self.changed();
        (ReplyPayload::Ack, outbox)
    }

    /// The host removes one member (BO2's `PartyHost_KickPlayer`): that member's
    /// session closes as `Kicked` and the rest see the new lobby.
    fn kick(&mut self, link: Link, member_id: RoomOccupantId) -> Answer {
        let lobby_number = match self.hosted(link) {
            Ok(lobby_number) => lobby_number,
            Err(error) => return refuse(error),
        };
        let Some(table) = self.tables.get(&lobby_number) else {
            return refuse(DirectoryServiceFault::NoSuchListing);
        };
        let Some(&target) = table.link_by_member.get(&member_id) else {
            return refuse(DirectoryServiceFault::NotRoomOccupant);
        };
        if target == link {
            return refuse(DirectoryServiceFault::GarbledRequest);
        }
        let epoch = table.view.epoch;
        let mut outbox = self.unseat(target);
        outbox.push((
            target,
            ControlEnvelope::Closed {
                lobby_number,
                epoch,
                reason: RoomShutdownCause::Kicked,
            },
        ));
        (ReplyPayload::Ack, outbox)
    }

    /// A member's admission news, stamped with its member id, for the host only.
    fn tell_host(&self, link: Link, event: impl FnOnce(RoomOccupantId) -> RoomMemberNotice) -> Answer {
        let (lobby_number, member_id) = match self.seat(link) {
            Ok(seat) => seat,
            Err(error) => return refuse(error),
        };
        let Some(table) = self.tables.get(&lobby_number) else {
            return refuse(DirectoryServiceFault::NoSuchListing);
        };
        let frame = ControlEnvelope::RoomMemberNotice(event(member_id));
        (ReplyPayload::Ack, vec![(table.host_link, frame)])
    }

    /// The host's admission news for one of its members.
    fn tell_member(&self, link: Link, member_id: RoomOccupantId, event: RoomMemberNotice) -> Answer {
        let lobby_number = match self.hosted(link) {
            Ok(lobby_number) => lobby_number,
            Err(error) => return refuse(error),
        };
        let Some(table) = self.tables.get(&lobby_number) else {
            return refuse(DirectoryServiceFault::NoSuchListing);
        };
        match table.link_by_member.get(&member_id) {
            Some(&to) => (ReplyPayload::Ack, vec![(to, ControlEnvelope::RoomMemberNotice(event))]),
            None => refuse(DirectoryServiceFault::NotRoomOccupant),
        }
    }

    /// Up to 128 rooms, by id.
    fn listing(&self) -> ReplyPayload {
        let mut adverts: Vec<RoomListing> = self.tables.values().map(|table| table.view.advert()).collect();
        adverts.sort_unstable_by_key(|advert| advert.id);
        adverts.truncate(LISTING_PAGE_LIMIT);
        ReplyPayload::RoomList {
            generation: self.list_stamp,
            adverts,
        }
    }

    /// Removes a room; every member hears why.
    fn close(&mut self, lobby_number: ListingId, reason: RoomShutdownCause) -> Outbox {
        let Some(table) = self.tables.remove(&lobby_number) else {
            return Outbox::new();
        };
        self.changed();
        let notice = ControlEnvelope::Closed {
            lobby_number,
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
        let Some(lobby_number) = self.seated_at.remove(&link) else {
            return Outbox::new();
        };
        let Some(table) = self.tables.get_mut(&lobby_number) else {
            return Outbox::new();
        };
        let Some(member_id) = table.member_by_link.remove(&link) else {
            return Outbox::new();
        };
        table.link_by_member.remove(&member_id);
        table.view.members.retain(|&kept| kept != member_id);
        table.view.member_names.remove(&member_id);
        table.bump_revision();
        let outbox = table.view_for_all();
        self.changed();
        outbox
    }

    fn changed(&mut self) {
        self.list_stamp = self.list_stamp.wrapping_add(1);
    }

    fn name_of(&self, link: Link) -> String {
        self.player_names
            .get(&link)
            .cloned()
            .unwrap_or_else(|| FALLBACK_NAME.to_owned())
    }
}

fn admission_refusal(member_id: RoomOccupantId, epoch: u32, reason: EntryRefusalReason) -> RoomMemberNotice {
    RoomMemberNotice::EntryRefused {
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

    fn create(lobby: &mut Lobby, link: Link) -> (ListingId, RoomOccupantId) {
        let (reply, _) = lobby.answer(
            link,
            QueryPayload::OpenLobby {
                name: "room".into(),
                map: "mp_raid".into(),
                mode: "tdm".into(),
                seat_capacity: 3,
                requires: AssetCoverageBits(0),
            },
        );
        match reply {
            ReplyPayload::LobbyOpened { member_id: host, view } => (view.lobby_number, host),
            other => panic!("create refused: {other:?}"),
        }
    }

    fn join(lobby: &mut Lobby, link: Link, lobby_number: ListingId) -> RoomOccupantId {
        let (reply, _) = lobby.answer(
            link,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0),
            },
        );
        match reply {
            ReplyPayload::LobbyEntered { member_id, .. } => member_id,
            other => panic!("join refused: {other:?}"),
        }
    }

    fn sorted(mut outbox: Outbox) -> Outbox {
        outbox.sort_by_key(|(link, _)| *link);
        outbox
    }

    fn list_stamp(lobby: &Lobby) -> u64 {
        match lobby.listing() {
            ReplyPayload::RoomList { generation: stamp, .. } => stamp,
            other => panic!("{other:?}"),
        }
    }

    fn view_of(lobby: &Lobby, lobby_number: ListingId) -> LobbyOverview {
        lobby.tables[&lobby_number].view.clone()
    }

    fn start(lobby: &mut Lobby) -> Answer {
        lobby.answer(
            HOST,
            QueryPayload::BeginGame {
                map: "mp_raid".into(),
                mode: "tdm".into(),
            },
        )
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(shown_label("  Ann \t"), "Ann");
        assert_eq!(shown_label("A\u{7}n\nn"), "Ann");
        assert_eq!(shown_label("abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnop");
        assert_eq!(shown_label(" \r\n "), "Player");
        assert_eq!(shown_label(""), "Player");
    }

    #[test]
    fn a_new_room_has_only_its_host() {
        let mut lobby = lobby();
        let (answer, outbox) = lobby.answer(
            HOST,
            QueryPayload::OpenLobby {
                name: "Ann's room".into(),
                map: "mp_raid".into(),
                mode: "tdm".into(),
                seat_capacity: 12,
                requires: AssetCoverageBits(2),
            },
        );
        let ReplyPayload::LobbyOpened { member_id: founder, view } = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(view.lobby_number, ListingId(nth(1)));
        assert_eq!(founder, RoomOccupantId(nth(2)));
        assert_eq!(view.host, founder);
        assert_eq!(view.members, vec![founder]);
        assert_eq!(view.member_names[&founder], "Ann");
        assert!(view.open_to_join);
        assert_eq!((view.revision, view.epoch), (1, 0));
        assert_eq!(view.phase, LobbyStage::Gathering);
        assert_eq!((view.seat_capacity, view.requires), (12, AssetCoverageBits(2)));
        assert_eq!(outbox, vec![(HOST, ControlEnvelope::LobbyOverview(view.clone()))]);
        assert_eq!(lobby.seat(HOST), Ok((view.lobby_number, founder)));
        assert_eq!(list_stamp(&lobby), 1);
    }

    #[test]
    fn one_room_per_connection() {
        let mut lobby = lobby();
        let (lobby_number, _) = create(&mut lobby, HOST);
        let (again, _) = lobby.answer(
            HOST,
            QueryPayload::OpenLobby {
                name: "x".into(),
                map: "m".into(),
                mode: "d".into(),
                seat_capacity: 2,
                requires: AssetCoverageBits(0),
            },
        );
        assert_eq!(again, ReplyPayload::Error(DirectoryServiceFault::HostSlotTaken));
        let (other, _) = create(&mut lobby, GUEST);
        let (cross, _) = lobby.answer(
            GUEST,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0),
            },
        );
        assert_ne!(other, lobby_number);
        assert_eq!(cross, ReplyPayload::Error(DirectoryServiceFault::HostSlotTaken));
    }

    #[test]
    fn room_limit() {
        let mut lobby = lobby();
        for link in 100..100 + ROOM_LIMIT as Link {
            create(&mut lobby, link);
        }
        let (answer, outbox) = lobby.answer(
            HOST,
            QueryPayload::OpenLobby {
                name: "late".into(),
                map: "m".into(),
                mode: "d".into(),
                seat_capacity: 2,
                requires: AssetCoverageBits(0),
            },
        );
        assert_eq!(answer, ReplyPayload::Error(DirectoryServiceFault::ListingsFull));
        assert!(outbox.is_empty());
    }

    #[test]
    fn joining_adds_a_member_everyone_sees() {
        let mut lobby = lobby();
        let (lobby_number, host) = create(&mut lobby, HOST);
        let (answer, outbox) = lobby.answer(
            GUEST,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0),
            },
        );
        let ReplyPayload::LobbyEntered { member_id: newcomer, view } = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(view.members, vec![host, newcomer]);
        assert_eq!(view.member_names[&newcomer], "Bo");
        assert_eq!(view.revision, 2);
        assert_eq!(list_stamp(&lobby), 2);
        let frame = ControlEnvelope::LobbyOverview(view.clone());
        assert_eq!(sorted(outbox), vec![(HOST, frame.clone()), (GUEST, frame)]);

        // Asking again changes nothing and tells nobody.
        let (again, outbox) = lobby.answer(
            GUEST,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0),
            },
        );
        assert_eq!(again, ReplyPayload::LobbyEntered { member_id: newcomer, view });
        assert!(outbox.is_empty());
        assert_eq!(list_stamp(&lobby), 2);
    }

    #[test]
    fn join_refusals() {
        let mut lobby = lobby();
        let (unknown, _) = lobby.answer(
            GUEST,
            QueryPayload::JoinRoom {
                lobby_number: ListingId([9; 16]),
                have: AssetCoverageBits(0),
            },
        );
        assert_eq!(unknown, ReplyPayload::Error(DirectoryServiceFault::NoSuchListing));

        let (_, outbox) = lobby.answer(
            HOST,
            QueryPayload::OpenLobby {
                name: "dlc".into(),
                map: "m".into(),
                mode: "d".into(),
                seat_capacity: 2,
                requires: AssetCoverageBits(0b101),
            },
        );
        let lobby_number = view_from(&outbox).lobby_number;
        let (missing, _) = lobby.answer(
            GUEST,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0b001),
            },
        );
        assert_eq!(
            missing,
            ReplyPayload::Error(DirectoryServiceFault::AbsentResource {
                requires: AssetCoverageBits(0b101),
                have: AssetCoverageBits(0b001),
            })
        );
        let (joined, _) = lobby.answer(
            GUEST,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0b111),
            },
        );
        assert!(matches!(joined, ReplyPayload::LobbyEntered { .. }));
        let (full, _) = lobby.answer(
            THIRD,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0b111),
            },
        );
        assert_eq!(full, ReplyPayload::Error(DirectoryServiceFault::Full));

        let (_, _) = lobby.answer(
            HOST,
            QueryPayload::ConfigureLobby {
                map: "m".into(),
                mode: "d".into(),
                open_to_join: false,
            },
        );
        let (_, _) = lobby.answer(GUEST, QueryPayload::LeaveRoom);
        let (locked, _) = lobby.answer(
            THIRD,
            QueryPayload::JoinRoom {
                lobby_number,
                have: AssetCoverageBits(0b111),
            },
        );
        assert_eq!(locked, ReplyPayload::Error(DirectoryServiceFault::Locked));
    }

    fn view_from(outbox: &Outbox) -> LobbyOverview {
        match &outbox[0].1 {
            ControlEnvelope::LobbyOverview(view) => view.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_member_leaving_updates_the_rest() {
        let mut lobby = lobby();
        let (lobby_number, host) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, lobby_number);
        let third = join(&mut lobby, THIRD, lobby_number);
        let (answer, outbox) = lobby.answer(GUEST, QueryPayload::LeaveRoom);
        assert_eq!(answer, ReplyPayload::RoomLeft);
        let view = view_of(&lobby, lobby_number);
        assert_eq!(view.members, vec![host, third]);
        assert_eq!(view.member_names.len(), 2);
        assert_eq!(view.revision, 4);
        let frame = ControlEnvelope::LobbyOverview(view);
        assert_eq!(sorted(outbox), vec![(HOST, frame.clone()), (THIRD, frame)]);
        assert_eq!(lobby.seat(GUEST), Err(DirectoryServiceFault::NotRoomOccupant));
        assert_eq!(list_stamp(&lobby), 4);

        let (idle, outbox) = lobby.answer(GUEST, QueryPayload::LeaveRoom);
        assert_eq!(idle, ReplyPayload::RoomLeft);
        assert!(outbox.is_empty());
    }

    #[test]
    fn the_host_leaving_closes_the_room() {
        for how in 0..3 {
            let mut lobby = lobby();
            let (lobby_number, _) = create(&mut lobby, HOST);
            join(&mut lobby, GUEST, lobby_number);
            let outbox = match how {
                0 => lobby.answer(HOST, QueryPayload::LeaveRoom).1,
                1 => {
                    let (answer, outbox) = lobby.answer(HOST, QueryPayload::CloseRoom);
                    assert_eq!(answer, ReplyPayload::Ack);
                    outbox
                }
                _ => lobby.depart(HOST),
            };
            let notice = ControlEnvelope::Closed {
                lobby_number,
                epoch: 0,
                reason: RoomShutdownCause::HostLeft,
            };
            assert_eq!(sorted(outbox), vec![(HOST, notice.clone()), (GUEST, notice)]);
            assert_eq!(lobby.seat(GUEST), Err(DirectoryServiceFault::NotRoomOccupant));
            assert_eq!(list_stamp(&lobby), 3);
            let ReplyPayload::RoomList { adverts, .. } = lobby.listing() else {
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
        let (lobby_number, host) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, lobby_number);
        let outbox = lobby.depart(GUEST);
        let view = view_of(&lobby, lobby_number);
        assert_eq!(view.members, vec![host]);
        assert_eq!(outbox, vec![(HOST, ControlEnvelope::LobbyOverview(view))]);
        assert!(lobby.depart(GUEST).is_empty());
        assert!(lobby.depart(99).is_empty());
    }

    #[test]
    fn only_the_host_changes_options() {
        let mut lobby = lobby();
        let options = || QueryPayload::ConfigureLobby {
            map: "mp_slums".into(),
            mode: "dom".into(),
            open_to_join: false,
        };
        assert_eq!(
            lobby.answer(HOST, options()).0,
            ReplyPayload::Error(DirectoryServiceFault::NotRoomOccupant)
        );
        let (lobby_number, _) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, lobby_number);
        assert_eq!(
            lobby.answer(GUEST, options()).0,
            ReplyPayload::Error(DirectoryServiceFault::NotRoomOwner)
        );
        let (answer, outbox) = lobby.answer(HOST, options());
        let view = view_of(&lobby, lobby_number);
        assert_eq!((view.map.as_str(), view.mode.as_str()), ("mp_slums", "dom"));
        assert!(!view.open_to_join);
        assert_eq!(view.revision, 3);
        assert_eq!(answer, ReplyPayload::RoomUpdated { view: view.clone() });
        assert_eq!(outbox.len(), 2);
        assert_eq!(list_stamp(&lobby), 3);
    }

    #[test]
    fn starting_a_match() {
        let mut lobby = lobby();
        let (lobby_number, _) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, lobby_number);
        let (answer, outbox) = start(&mut lobby);
        let view = view_of(&lobby, lobby_number);
        assert_eq!((view.epoch, view.phase, view.revision), (1, LobbyStage::Loading, 3));
        assert_eq!(answer, ReplyPayload::RoomUpdated { view: view.clone() });
        assert_eq!(outbox.len(), 2);

        // Again with the same settings: answered, nothing changes.
        let (answer, outbox) = start(&mut lobby);
        assert_eq!(answer, ReplyPayload::RoomUpdated { view: view.clone() });
        assert!(outbox.is_empty());
        assert_eq!(list_stamp(&lobby), 3);

        // The map and mode are fixed for the match, in either request.
        let moved = lobby.answer(
            HOST,
            QueryPayload::BeginGame {
                map: "mp_nuketown_2020".into(),
                mode: "tdm".into(),
            },
        );
        assert_eq!(moved.0, ReplyPayload::Error(DirectoryServiceFault::ListingRejected));
        let moved = lobby.answer(
            HOST,
            QueryPayload::ConfigureLobby {
                map: "mp_raid".into(),
                mode: "sd".into(),
                open_to_join: true,
            },
        );
        assert_eq!(moved.0, ReplyPayload::Error(DirectoryServiceFault::ListingRejected));
        // Locking the room mid-match is fine.
        let locked = lobby.answer(
            HOST,
            QueryPayload::ConfigureLobby {
                map: "mp_raid".into(),
                mode: "tdm".into(),
                open_to_join: false,
            },
        );
        assert!(matches!(locked.0, ReplyPayload::RoomUpdated { .. }));
        assert_eq!(view_of(&lobby, lobby_number).revision, 4);
    }

    #[test]
    fn world_ready_then_end() {
        let mut lobby = lobby();
        let (lobby_number, _) = create(&mut lobby, HOST);
        join(&mut lobby, GUEST, lobby_number);
        let ready = |lobby_number, epoch| QueryPayload::HostMapLoaded {
            lobby_number,
            epoch,
            map: 11,
            weapons: 22,
            classes: 33,
        };
        // Before any match, and for the wrong match: acknowledged, ignored.
        assert_eq!(lobby.answer(HOST, ready(lobby_number, 0)), (ReplyPayload::Ack, Outbox::new()));
        start(&mut lobby);
        assert_eq!(lobby.answer(HOST, ready(lobby_number, 2)), (ReplyPayload::Ack, Outbox::new()));
        assert_eq!(
            lobby.answer(HOST, ready(ListingId([7; 16]), 1)),
            (ReplyPayload::Ack, Outbox::new())
        );
        assert_eq!(
            lobby.answer(GUEST, ready(lobby_number, 1)).0,
            ReplyPayload::Error(DirectoryServiceFault::NotRoomOwner)
        );

        let (answer, outbox) = lobby.answer(HOST, ready(lobby_number, 1));
        assert_eq!(answer, ReplyPayload::Ack);
        let view = view_of(&lobby, lobby_number);
        assert_eq!((view.phase, view.revision), (LobbyStage::Running, 4));
        let event = ControlEnvelope::RoomMemberNotice(RoomMemberNotice::HostMapLoaded {
            epoch: 1,
            map: 11,
            weapon_digest: 22,
            class_digest: 33,
        });
        // The views go out before the event.
        assert!(outbox[..2]
            .iter()
            .all(|(_, frame)| matches!(frame, ControlEnvelope::LobbyOverview(_))));
        let mut events: Vec<_> = outbox[2..].to_vec();
        events.sort_by_key(|(link, _)| *link);
        assert_eq!(events, vec![(HOST, event.clone()), (GUEST, event)]);

        let end = |epoch| QueryPayload::EndMatch { lobby_number, epoch };
        assert_eq!(lobby.answer(HOST, end(5)), (ReplyPayload::Ack, Outbox::new()));
        let (answer, outbox) = lobby.answer(HOST, end(1));
        assert_eq!(answer, ReplyPayload::Ack);
        assert_eq!(outbox.len(), 2);
        let view = view_of(&lobby, lobby_number);
        assert_eq!((view.phase, view.epoch, view.revision), (LobbyStage::Gathering, 1, 5));
        // Already ended.
        assert_eq!(lobby.answer(HOST, end(1)), (ReplyPayload::Ack, Outbox::new()));
        // The next match is number 2.
        start(&mut lobby);
        assert_eq!(view_of(&lobby, lobby_number).epoch, 2);
    }

    #[test]
    fn member_news_goes_to_the_host() {
        let mut lobby = lobby();
        let (lobby_number, _) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, lobby_number);
        let (answer, outbox) = lobby.answer(
            GUEST,
            QueryPayload::MapLoaded {
                epoch: 1,
                map: 4,
                weapons: 5,
                classes: 6,
            },
        );
        assert_eq!(answer, ReplyPayload::Ack);
        assert_eq!(
            outbox,
            vec![(
                HOST,
                ControlEnvelope::RoomMemberNotice(RoomMemberNotice::MapLoaded {
                    member_id: guest,
                    epoch: 1,
                    map: 4,
                    weapon_digest: 5,
                    class_digest: 6,
                })
            )]
        );
        let (_, outbox) = lobby.answer(
            GUEST,
            QueryPayload::TransferInstalledMsg {
                epoch: 1,
                bootstrap_id: 7,
                frame_number_on_wire: 8,
                connection: 9,
            },
        );
        assert_eq!(
            outbox,
            vec![(
                HOST,
                ControlEnvelope::RoomMemberNotice(RoomMemberNotice::TransferInstalledMsg {
                    member_id: guest,
                    epoch: 1,
                    transfer_id: 7,
                    frame_number_on_wire: 8,
                    connection: 9,
                })
            )]
        );
        let (_, outbox) = lobby.answer(GUEST, QueryPayload::BallotForSkip);
        assert_eq!(
            outbox,
            vec![(HOST, ControlEnvelope::RoomMemberNotice(RoomMemberNotice::BallotForSkip { member_id: guest }))]
        );
        assert_eq!(
            lobby.answer(THIRD, QueryPayload::BallotForSkip),
            (ReplyPayload::Error(DirectoryServiceFault::NotRoomOccupant), Outbox::new())
        );
    }

    #[test]
    fn the_host_kicks_one_member() {
        let mut lobby = lobby();
        let (lobby_number, host_member) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, lobby_number);
        join(&mut lobby, THIRD, lobby_number);
        assert_eq!(
            lobby.answer(GUEST, QueryPayload::KickMember { member_id: guest }).0,
            ReplyPayload::Error(DirectoryServiceFault::NotRoomOwner)
        );
        assert_eq!(
            lobby.answer(HOST, QueryPayload::KickMember { member_id: host_member }).0,
            ReplyPayload::Error(DirectoryServiceFault::GarbledRequest)
        );
        let (answer, outbox) = lobby.answer(HOST, QueryPayload::KickMember { member_id: guest });
        assert_eq!(answer, ReplyPayload::Ack);
        assert!(outbox.iter().any(|(to, frame)| *to == GUEST
            && matches!(
                frame,
                ControlEnvelope::Closed {
                    reason: RoomShutdownCause::Kicked,
                    ..
                }
            )));
        assert!(outbox.iter().any(|(to, _)| *to == THIRD));
        assert_eq!(lobby.seat(GUEST), Err(DirectoryServiceFault::NotRoomOccupant));
    }

    #[test]
    fn host_news_goes_to_one_member() {
        let mut lobby = lobby();
        let (lobby_number, _) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, lobby_number);
        join(&mut lobby, THIRD, lobby_number);
        let enter = |member_id| QueryPayload::JoinGame {
            member_id,
            epoch: 1,
            bootstrap_id: 2,
            connection_id: 3,
            player_slot_no: 4,
        };
        let (answer, outbox) = lobby.answer(HOST, enter(guest));
        assert_eq!(answer, ReplyPayload::Ack);
        assert_eq!(
            outbox,
            vec![(
                GUEST,
                ControlEnvelope::RoomMemberNotice(RoomMemberNotice::JoinGame {
                    member_id: guest,
                    epoch: 1,
                    transfer_id: 2,
                    link_id: 3,
                    player_slot_no: 4,
                })
            )]
        );
        let (_, outbox) = lobby.answer(
            HOST,
            QueryPayload::EntryRefused {
                member_id: guest,
                epoch: 1,
                reason: EntryRefusalReason::CutoffInstant,
            },
        );
        assert_eq!(
            outbox,
            vec![(
                GUEST,
                ControlEnvelope::RoomMemberNotice(RoomMemberNotice::EntryRefused {
                    member_id: guest,
                    epoch: 1,
                    reason: EntryRefusalReason::CutoffInstant,
                })
            )]
        );
        assert_eq!(
            lobby.answer(HOST, enter(RoomOccupantId([0xee; 16]))).0,
            ReplyPayload::Error(DirectoryServiceFault::NotRoomOccupant)
        );
        assert_eq!(
            lobby.answer(GUEST, enter(guest)).0,
            ReplyPayload::Error(DirectoryServiceFault::NotRoomOwner)
        );
    }

    #[test]
    fn packets_route_through_the_room() {
        let mut lobby = lobby();
        let (lobby_number, _) = create(&mut lobby, HOST);
        let guest = join(&mut lobby, GUEST, lobby_number);
        let up = ForwardedPacket::ReceiverToHost(b"up");
        assert_eq!(
            lobby.route(GUEST, &up),
            Ok(Hop {
                to: HOST,
                to_host: true,
                packet: ForwardedPacket::DirectoryToHost {
                    member_id: guest,
                    payload: b"up",
                },
            })
        );
        let down = ForwardedPacket::HostToParticipant {
            member_id: guest,
            payload: b"down",
        };
        assert_eq!(
            lobby.route(HOST, &down),
            Ok(Hop {
                to: GUEST,
                to_host: false,
                packet: ForwardedPacket::DirectoryToParticipant(b"down"),
            })
        );
        assert_eq!(lobby.route(THIRD, &up), Err(DirectoryServiceFault::NotRoomOccupant));
        assert_eq!(lobby.route(GUEST, &down), Err(DirectoryServiceFault::NotRoomOwner));
        let stranger = ForwardedPacket::HostToParticipant {
            member_id: RoomOccupantId([0xee; 16]),
            payload: b"",
        };
        assert_eq!(lobby.route(HOST, &stranger), Err(DirectoryServiceFault::NotRoomOccupant));
        assert_eq!(
            lobby.route(HOST, &ForwardedPacket::DirectoryToParticipant(b"")),
            Err(DirectoryServiceFault::GarbledRequest)
        );
    }

    #[test]
    fn the_list_is_sorted_and_capped() {
        let mut lobby = lobby();
        for link in 10..10 + LISTING_PAGE_LIMIT as Link + 5 {
            create(&mut lobby, link);
        }
        let ReplyPayload::RoomList {
            generation: stamp,
            adverts,
        } = lobby.listing()
        else {
            unreachable!()
        };
        assert_eq!(stamp, LISTING_PAGE_LIMIT as u64 + 5);
        assert_eq!(adverts.len(), LISTING_PAGE_LIMIT);
        assert!(adverts.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_eq!(adverts[0].players, 1);
    }
}
