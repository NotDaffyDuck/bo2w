//! What the launcher asked the lobby link to do, read from the environment.
//!
//! `BO2MP_MASTER_ADDR` switches the link on; without it the game runs alone.
//! With it, `BO2MP_MASTER_SERVER_NAME` (the certificate name) is required and
//! `BO2MP_MASTER_CA_CERT` optionally names a PEM file of trusted roots.
//!
//! A LAN host needs no master: `BO2MP_MASTER_ADDR=lan` (or `lan:<port>`), or a
//! `BO2MP_MASTER_HOST_NAME` with no address at all, makes this process run the
//! lobby service itself on UDP 4433 and dial it. The other PCs then set
//! `BO2MP_MASTER_ADDR=<host's address>:4433`, `BO2MP_MASTER_SERVER_NAME=bo2mp-lan`
//! and `BO2MP_MASTER_CA_CERT` to a copy of the host's `lan/bo2mp-lan.pem`.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use bevy::prelude::Resource;
use bo2mp_master::lan;
use master_protocol::{ListingId, AssetCoverageBits, ROOM_OCCUPANT_LIMIT};

const ADDRESS: &str = "BO2MP_MASTER_ADDR";
const SERVER_NAME: &str = "BO2MP_MASTER_SERVER_NAME";
const CA_FILE: &str = "BO2MP_MASTER_CA_CERT";
const HOST_NAME: &str = "BO2MP_MASTER_HOST_NAME";
const JOIN_ROOM: &str = "BO2MP_MASTER_JOIN";
const GAME_MODE: &str = "BO2MP_GAMETYPE";
const ROOM_SIZE: &str = "BO2MP_MASTER_MAX_PLAYERS";
const DEFAULT_MODE: &str = "dm";
/// The room name a menu-hosted room gets when the environment names none.
const MENU_ROOM_NAME: &str = "bo2mp host";
/// The `BO2MP_MASTER_ADDR` word for "be the lobby service yourself".
const LAN_WORD: &str = "lan";
/// The folder beside the exe that keeps the LAN certificate.
const LAN_FOLDER: &str = "lan";

/// Where the lobby service is and how its certificate is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Dial {
    pub address: String,
    pub server_name: String,
    pub ca_file: Option<PathBuf>,
    /// A LAN host: open the lobby service on this address first, then dial it.
    pub serve_here: Option<SocketAddr>,
}

impl Dial {
    /// This process as the lobby service on `port`: listen on every interface,
    /// dial it on loopback, and trust the certificate kept beside the exe.
    fn lan(port: u16) -> Self {
        let folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(LAN_FOLDER)))
            .unwrap_or_else(|| PathBuf::from(LAN_FOLDER));
        Self {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)).to_string(),
            server_name: lan::SERVER_NAME.to_owned(),
            ca_file: Some(folder.join(lan::CERT_FILE)),
            serve_here: Some(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HostPlan {
    pub dial: Dial,
    /// Launched straight into a map: open the match as soon as the world is live.
    pub start_on_load: bool,
    pub room_name: String,
    pub map: String,
    pub ruleset: String,
    pub room_size: u8,
    pub requires: AssetCoverageBits,
    pub have: AssetCoverageBits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JoinPlan {
    pub dial: Dial,
    pub room: ListingId,
    pub map: String,
    pub ruleset: String,
    pub have: AssetCoverageBits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    Off,
    Browse { dial: Dial, have: AssetCoverageBits },
    Host(HostPlan),
    Join(JoinPlan),
}

#[derive(Resource, Clone, Debug)]
pub struct DirectoryLaunchRequest(pub(super) Plan);

/// Reads one environment variable; absent and non-Unicode both read as `None`
/// for text, while paths keep their raw bytes.

fn text(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> Option<String> {
    env(name).and_then(|raw| raw.into_string().ok())
}

/// `None` when the link is off; an error when it is on but half configured.
fn dial_from(env: &dyn Fn(&str) -> Option<OsString>) -> Result<Option<Dial>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(address) = text(env, ADDRESS) else {
        return Ok(None);
    };
    if address == LAN_WORD {
        return Ok(Some(Dial::lan(lan::PORT)));
    }
    if let Some(port) = address.strip_prefix(LAN_WORD).and_then(|rest| rest.strip_prefix(':')) {
        return Ok(Some(Dial::lan(port.parse().map_err(|_| format!("{ADDRESS}={address}: bad port"))?)));
    }
    let tls_name =
        text(env, SERVER_NAME).ok_or("BO2MP_MASTER_ADDR requires BO2MP_MASTER_SERVER_NAME")?;
    Ok(Some(Dial {
        address,
        server_name: tls_name,
        ca_file: env(CA_FILE).map(PathBuf::from),
        serve_here: None,
    }))
}

pub(super) fn room_size_from(env: &dyn Fn(&str) -> Option<OsString>) -> Result<u8, Box<dyn std::error::Error + Send + Sync>> {
    let Some(raw) = text(env, ROOM_SIZE) else {
        return Ok(ROOM_OCCUPANT_LIMIT);
    };
    let size: u8 = raw.parse()?;
    if size < 2 || size > ROOM_OCCUPANT_LIMIT {
        return Err(format!("{ROOM_SIZE}={size} must be 2..={ROOM_OCCUPANT_LIMIT}").into());
    }
    Ok(size)
}

fn mode_from(env: &dyn Fn(&str) -> Option<OsString>) -> String {
    text(env, GAME_MODE).unwrap_or_else(|| DEFAULT_MODE.to_owned())
}

fn browse_plan(env: &dyn Fn(&str) -> Option<OsString>, have: AssetCoverageBits) -> Result<Plan, Box<dyn std::error::Error + Send + Sync>> {
    Ok(match dial_from(env)? {
        Some(dial) => Plan::Browse { dial, have },
        None => Plan::Off,
    })
}

fn launch_plan(
    env: &dyn Fn(&str) -> Option<OsString>,
    map: &str,
    have: AssetCoverageBits,
    requires: AssetCoverageBits,
) -> Result<Plan, Box<dyn std::error::Error + Send + Sync>> {
    let host_name = text(env, HOST_NAME);
    let join_room = text(env, JOIN_ROOM);
    let names_a_room = host_name.as_deref().is_some_and(|name| !name.trim().is_empty());
    // Hosting with no master to go through: this PC is the lobby service.
    let dial = match dial_from(env)? {
        Some(dial) => dial,
        None if names_a_room && join_room.is_none() => Dial::lan(lan::PORT),
        None => return Ok(Plan::Off),
    };
    if host_name.is_some() && join_room.is_some() {
        return Err("set only one of BO2MP_MASTER_HOST_NAME or BO2MP_MASTER_JOIN".into());
    }
    if let Some(room) = join_room {
        return Ok(Plan::Join(JoinPlan {
            dial,
            room: room.parse()?,
            map: map.to_owned(),
            ruleset: mode_from(env),
            have,
        }));
    }
    match host_name {
        Some(room_name) if !room_name.trim().is_empty() => Ok(Plan::Host(HostPlan {
            dial,
            start_on_load: true,
            room_name,
            map: map.to_owned(),
            ruleset: mode_from(env),
            room_size: room_size_from(env)?,
            requires,
            have,
        })),
        // Black Ops II's front end looks the relay's games up itself (Public Match joins one or
        // hosts its own).
        None if ::frame::bo2_frontend().is_some() => Ok(Plan::Browse { dial, have }),
        _ => Err("BO2MP_MASTER_ADDR requires BO2MP_MASTER_HOST_NAME or BO2MP_MASTER_JOIN".into()),
    }
}

pub(super) fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

impl DirectoryLaunchRequest {
    /// The front-end server browser, or off when no lobby service is set.
    pub fn browser_from_env(have: AssetCoverageBits) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        browse_plan(&process_env, have).map(Self)
    }

    /// A launch straight into `map`: host a room or join one, per the environment.
    pub fn from_env_for_map(
        map: &str,
        have: AssetCoverageBits,
        requires: AssetCoverageBits,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        launch_plan(&process_env, map, have, requires).map(Self)
    }

    pub const fn disabled() -> Self {
        Self(Plan::Off)
    }

    pub const fn denotes_arrival(&self) -> bool {
        matches!(self.0, Plan::Join(_))
    }

    pub const fn enabled(&self) -> bool {
        !matches!(self.0, Plan::Off)
    }

    pub const fn configured(&self) -> bool {
        self.enabled()
    }

    pub(super) fn dial(&self) -> Option<&Dial> {
        match &self.0 {
            Plan::Off => None,
            Plan::Browse { dial, .. } => Some(dial),
            Plan::Host(host) => Some(&host.dial),
            Plan::Join(join) => Some(&join.dial),
        }
    }

    pub(super) fn have(&self) -> AssetCoverageBits {
        match &self.0 {
            Plan::Off => AssetCoverageBits::default(),
            Plan::Browse { have, .. } => *have,
            Plan::Host(host) => host.have,
            Plan::Join(join) => join.have,
        }
    }

    /// The service and installed packs while the menu shows the browser.
    pub(super) fn browsing(&self) -> Option<(&Dial, AssetCoverageBits)> {
        match &self.0 {
            Plan::Browse { dial, have } => Some((dial, *have)),
            _ => None,
        }
    }

    /// Back to the server browser on the same service; a link that was off stays off.
    pub(super) fn back_to_browser(&mut self) {
        let have = self.have();
        if let Some(dial) = self.dial().cloned() {
            self.0 = Plan::Browse { dial, have };
        }
    }
}

/// A room the player chose to host from the menu: it opens in the lobby, not
/// straight into a match.
pub(super) fn menu_host(
    env: &dyn Fn(&str) -> Option<OsString>,
    dial: &Dial,
    have: AssetCoverageBits,
    map: String,
    ruleset: String,
    requires: AssetCoverageBits,
) -> Result<Plan, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Plan::Host(HostPlan {
        dial: dial.clone(),
        start_on_load: false,
        room_name: text(env, HOST_NAME).unwrap_or_else(|| MENU_ROOM_NAME.to_owned()),
        // A BO2 Zombies lobby holds 4 ("1 Player (4 Max)").
        room_size: if map.starts_with("t6:") { room_size_from(env)?.min(4) } else { room_size_from(env)? },
        map,
        ruleset,
        requires,
        have,
    }))
}

/// A browser row the player chose to join.
pub(super) fn menu_join(dial: &Dial, have: AssetCoverageBits, room: ListingId, arena: String, ruleset: String) -> Plan {
    Plan::Join(JoinPlan { dial: dial.clone(), room, map: arena, ruleset, have })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const ROOM: &str = "00112233445566778899aabbccddeeff";

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, val)| ((*key).to_owned(), (*val).to_owned()))
            .collect();
        move |name| map.get(name).map(OsString::from)
    }

    fn launch(pairs: &[(&str, &str)]) -> Result<Plan, Box<dyn std::error::Error + Send + Sync>> {
        launch_plan(&env_of(pairs), "mp_x", AssetCoverageBits(8), AssetCoverageBits(8))
    }

    const DIAL: [(&str, &str); 2] = [(ADDRESS, "lobby:4433"), (SERVER_NAME, "lobby")];

    #[test]
    fn no_address_means_off() {
        assert_eq!(launch(&[]).unwrap(), Plan::Off);
        assert_eq!(browse_plan(&env_of(&[]), AssetCoverageBits(8)).unwrap(), Plan::Off);
    }

    #[test]
    fn a_host_name_without_a_master_hosts_on_the_lan() {
        let Plan::Host(host) = launch(&[(HOST_NAME, "my room")]).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!(host.room_name, "my room");
        assert_eq!(host.dial.address, "127.0.0.1:4433");
        assert_eq!(host.dial.server_name, "bo2mp-lan");
        assert_eq!(host.dial.serve_here, Some("0.0.0.0:4433".parse().unwrap()));
        let pem = host.dial.ca_file.expect("the LAN certificate");
        assert!(pem.ends_with(std::path::Path::new("lan").join("bo2mp-lan.pem")), "{}", pem.display());
        // A blank name, or a join with no host to dial, still runs alone.
        assert_eq!(launch(&[(HOST_NAME, " ")]).unwrap(), Plan::Off);
        assert_eq!(launch(&[(JOIN_ROOM, ROOM)]).unwrap(), Plan::Off);
    }

    #[test]
    fn the_lan_address_word_serves_here() {
        let Plan::Browse { dial, .. } = browse_plan(&env_of(&[(ADDRESS, "lan")]), AssetCoverageBits(4)).unwrap() else {
            panic!("expected a browse plan");
        };
        assert_eq!(dial.serve_here, Some("0.0.0.0:4433".parse().unwrap()));
        let Plan::Host(host) = launch(&[(ADDRESS, "lan:5000"), (HOST_NAME, "r")]).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!(host.dial.address, "127.0.0.1:5000");
        assert_eq!(host.dial.serve_here, Some("0.0.0.0:5000".parse().unwrap()));
        assert!(launch(&[(ADDRESS, "lan:x"), (HOST_NAME, "r")]).is_err());
        // A real master is never served here.
        let Plan::Host(host) = launch(&[DIAL[0], DIAL[1], (HOST_NAME, "r")]).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!(host.dial.serve_here, None);
    }

    #[test]
    fn address_without_server_name_is_refused() {
        let error = launch(&[(ADDRESS, "lobby:4433")]).unwrap_err();
        assert_eq!(error.to_string(), "BO2MP_MASTER_ADDR requires BO2MP_MASTER_SERVER_NAME");
        let error = browse_plan(&env_of(&[(ADDRESS, "a")]), AssetCoverageBits(0)).unwrap_err();
        assert_eq!(error.to_string(), "BO2MP_MASTER_ADDR requires BO2MP_MASTER_SERVER_NAME");
    }

    #[test]
    fn browse_keeps_the_ca_file() {
        let env = env_of(&[DIAL[0], DIAL[1], (CA_FILE, "roots.pem")]);
        let Plan::Browse { dial, have } = browse_plan(&env, AssetCoverageBits(4)).unwrap() else {
            panic!("expected a browse plan");
        };
        assert_eq!(dial.address, "lobby:4433");
        assert_eq!(dial.server_name, "lobby");
        assert_eq!(dial.ca_file, Some(PathBuf::from("roots.pem")));
        assert_eq!(have, AssetCoverageBits(4));
    }

    #[test]
    fn host_defaults_to_dm_and_a_full_room() {
        let Plan::Host(host) = launch(&[DIAL[0], DIAL[1], (HOST_NAME, "my room")]).unwrap() else {
            panic!("expected a host plan");
        };
        assert!(host.start_on_load);
        assert_eq!(host.room_name, "my room");
        assert_eq!(host.map, "mp_x");
        assert_eq!(host.ruleset, "dm");
        assert_eq!(host.room_size, ROOM_OCCUPANT_LIMIT);
        assert_eq!(host.dial.ca_file, None);
    }

    #[test]
    fn host_reads_mode_and_room_size() {
        let pairs = [DIAL[0], DIAL[1], (HOST_NAME, "r"), (GAME_MODE, "tdm"), (ROOM_SIZE, "6")];
        let Plan::Host(host) = launch(&pairs).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!(host.ruleset, "tdm");
        assert_eq!(host.room_size, 6);
    }

    #[test]
    fn room_size_outside_two_to_eighteen_is_refused() {
        for bad in ["1", "19"] {
            let error = launch(&[DIAL[0], DIAL[1], (HOST_NAME, "r"), (ROOM_SIZE, bad)]).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("BO2MP_MASTER_MAX_PLAYERS={bad} must be 2..=18")
            );
        }
        assert!(launch(&[DIAL[0], DIAL[1], (HOST_NAME, "r"), (ROOM_SIZE, "x")]).is_err());
    }

    #[test]
    fn join_parses_the_room_id() {
        let Plan::Join(join) = launch(&[DIAL[0], DIAL[1], (JOIN_ROOM, ROOM)]).unwrap() else {
            panic!("expected a join plan");
        };
        assert_eq!(join.room, ROOM.parse::<ListingId>().unwrap());
        assert_eq!(join.ruleset, "dm");
        assert!(launch(&[DIAL[0], DIAL[1], (JOIN_ROOM, "zz")]).is_err());
    }

    #[test]
    fn host_and_join_together_are_refused() {
        let error = launch(&[DIAL[0], DIAL[1], (HOST_NAME, "r"), (JOIN_ROOM, ROOM)]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "set only one of BO2MP_MASTER_HOST_NAME or BO2MP_MASTER_JOIN"
        );
    }

    #[test]
    fn neither_or_a_blank_host_name_is_refused() {
        let want = "BO2MP_MASTER_ADDR requires BO2MP_MASTER_HOST_NAME or BO2MP_MASTER_JOIN";
        assert_eq!(launch(&DIAL).unwrap_err().to_string(), want);
        let blank = launch(&[DIAL[0], DIAL[1], (HOST_NAME, "  ")]).unwrap_err();
        assert_eq!(blank.to_string(), want);
    }

    #[test]
    fn menu_host_opens_in_the_lobby() {
        let dial = Dial { address: "a:1".into(), server_name: "a".into(), ca_file: None, serve_here: None };
        let plain = menu_host(&env_of(&[]), &dial, AssetCoverageBits(4), "mp_y".into(), "tdm".into(), AssetCoverageBits(8));
        let Plan::Host(host) = plain.unwrap() else {
            panic!("expected a host plan");
        };
        assert!(!host.start_on_load);
        assert_eq!(host.room_name, MENU_ROOM_NAME);
        assert_eq!((host.map.as_str(), host.ruleset.as_str()), ("mp_y", "tdm"));
        assert_eq!((host.have, host.requires), (AssetCoverageBits(4), AssetCoverageBits(8)));
        assert_eq!(host.room_size, ROOM_OCCUPANT_LIMIT);
        let named = env_of(&[(HOST_NAME, ""), (ROOM_SIZE, "4")]);
        let Plan::Host(host) = menu_host(&named, &dial, AssetCoverageBits(0), "m".into(), "dm".into(), AssetCoverageBits(0)).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!((host.room_name.as_str(), host.room_size), ("", 4));
        let too_big = env_of(&[(ROOM_SIZE, "40")]);
        assert!(menu_host(&too_big, &dial, AssetCoverageBits(0), "m".into(), "dm".into(), AssetCoverageBits(0)).is_err());
    }

    #[test]
    fn leaving_a_room_goes_back_to_the_browser() {
        let mut join = DirectoryLaunchRequest(launch(&[DIAL[0], DIAL[1], (JOIN_ROOM, ROOM)]).unwrap());
        assert!(join.browsing().is_none());
        join.back_to_browser();
        let (dial, have) = join.browsing().expect("browsing again");
        assert_eq!((dial.address.as_str(), have), ("lobby:4433", AssetCoverageBits(8)));
        let room = ROOM.parse::<ListingId>().unwrap();
        let picked = menu_join(dial, have, room, "mp_z".into(), "dm".into());
        let Plan::Join(plan) = picked else {
            panic!("expected a join plan");
        };
        assert_eq!((plan.room, plan.have, plan.map.as_str()), (room, AssetCoverageBits(8), "mp_z"));
        let mut off = DirectoryLaunchRequest::disabled();
        off.back_to_browser();
        assert!(!off.enabled());
    }

    #[test]
    fn intent_flags() {
        let off = DirectoryLaunchRequest::disabled();
        assert!(!off.enabled() && !off.configured() && !off.denotes_arrival());
        let join = DirectoryLaunchRequest(launch(&[DIAL[0], DIAL[1], (JOIN_ROOM, ROOM)]).unwrap());
        assert!(join.enabled() && join.configured() && join.denotes_arrival());
        assert_eq!(join.have(), AssetCoverageBits(8));
        assert_eq!(join.dial().map(|dial| dial.server_name.as_str()), Some("lobby"));
    }
}
