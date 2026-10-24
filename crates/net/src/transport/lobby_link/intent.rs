//! What the launcher asked the lobby link to do, read from the environment.
//!
//! `BO2MP_MASTER_ADDR` switches the link on; without it the game runs alone.
//! With it, `BO2MP_MASTER_SERVER_NAME` (the certificate name) is required and
//! `BO2MP_MASTER_CA_CERT` optionally names a PEM file of trusted roots.

use std::ffi::OsString;
use std::path::PathBuf;

use bevy::prelude::Resource;
use master_protocol::{AdvertId, ContentFlags, MAX_SESSION_MEMBERS};

use super::BoxError;

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

/// Where the lobby service is and how its certificate is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Dial {
    pub address: String,
    pub server_name: String,
    pub ca_file: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HostPlan {
    pub dial: Dial,
    /// Launched straight into a map: open the match as soon as the world is live.
    pub start_on_load: bool,
    pub room_name: String,
    pub map: String,
    pub mode: String,
    pub room_size: u8,
    pub requires: ContentFlags,
    pub have: ContentFlags,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JoinPlan {
    pub dial: Dial,
    pub room: AdvertId,
    pub map: String,
    pub mode: String,
    pub have: ContentFlags,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    Off,
    Browse { dial: Dial, have: ContentFlags },
    Host(HostPlan),
    Join(JoinPlan),
}

#[derive(Resource, Clone, Debug)]
pub struct MasterLaunchIntent(pub(super) Plan);

/// Reads one environment variable; absent and non-Unicode both read as `None`
/// for text, while paths keep their raw bytes.
pub(super) type Lookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn text(env: Lookup<'_>, name: &str) -> Option<String> {
    env(name).and_then(|raw| raw.into_string().ok())
}

/// `None` when the link is off; an error when it is on but half configured.
fn dial_from(env: Lookup<'_>) -> Result<Option<Dial>, BoxError> {
    let Some(address) = text(env, ADDRESS) else {
        return Ok(None);
    };
    let server_name =
        text(env, SERVER_NAME).ok_or("BO2MP_MASTER_ADDR requires BO2MP_MASTER_SERVER_NAME")?;
    Ok(Some(Dial {
        address,
        server_name,
        ca_file: env(CA_FILE).map(PathBuf::from),
    }))
}

pub(super) fn room_size_from(env: Lookup<'_>) -> Result<u8, BoxError> {
    let Some(raw) = text(env, ROOM_SIZE) else {
        return Ok(MAX_SESSION_MEMBERS);
    };
    let size: u8 = raw.parse()?;
    if size < 2 || size > MAX_SESSION_MEMBERS {
        return Err(format!("{ROOM_SIZE}={size} must be 2..={MAX_SESSION_MEMBERS}").into());
    }
    Ok(size)
}

fn mode_from(env: Lookup<'_>) -> String {
    text(env, GAME_MODE).unwrap_or_else(|| DEFAULT_MODE.to_owned())
}

fn browse_plan(env: Lookup<'_>, have: ContentFlags) -> Result<Plan, BoxError> {
    Ok(match dial_from(env)? {
        Some(dial) => Plan::Browse { dial, have },
        None => Plan::Off,
    })
}

fn launch_plan(
    env: Lookup<'_>,
    map: &str,
    have: ContentFlags,
    requires: ContentFlags,
) -> Result<Plan, BoxError> {
    let Some(dial) = dial_from(env)? else {
        return Ok(Plan::Off);
    };
    let host_name = text(env, HOST_NAME);
    let join_room = text(env, JOIN_ROOM);
    if host_name.is_some() && join_room.is_some() {
        return Err("set only one of BO2MP_MASTER_HOST_NAME or BO2MP_MASTER_JOIN".into());
    }
    if let Some(room) = join_room {
        return Ok(Plan::Join(JoinPlan {
            dial,
            room: room.parse()?,
            map: map.to_owned(),
            mode: mode_from(env),
            have,
        }));
    }
    match host_name {
        Some(room_name) if !room_name.trim().is_empty() => Ok(Plan::Host(HostPlan {
            dial,
            start_on_load: true,
            room_name,
            map: map.to_owned(),
            mode: mode_from(env),
            room_size: room_size_from(env)?,
            requires,
            have,
        })),
        _ => Err("BO2MP_MASTER_ADDR requires BO2MP_MASTER_HOST_NAME or BO2MP_MASTER_JOIN".into()),
    }
}

pub(super) fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

impl MasterLaunchIntent {
    /// The front-end server browser, or off when no lobby service is set.
    pub fn browser_from_env(have: ContentFlags) -> Result<Self, BoxError> {
        browse_plan(&process_env, have).map(Self)
    }

    /// A launch straight into `map`: host a room or join one, per the environment.
    pub fn from_env_for_map(
        map: &str,
        have: ContentFlags,
        requires: ContentFlags,
    ) -> Result<Self, BoxError> {
        launch_plan(&process_env, map, have, requires).map(Self)
    }

    pub const fn disabled() -> Self {
        Self(Plan::Off)
    }

    pub const fn is_join(&self) -> bool {
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

    pub(super) fn have(&self) -> ContentFlags {
        match &self.0 {
            Plan::Off => ContentFlags::default(),
            Plan::Browse { have, .. } => *have,
            Plan::Host(host) => host.have,
            Plan::Join(join) => join.have,
        }
    }

    /// The service and installed packs while the menu shows the browser.
    pub(super) fn browsing(&self) -> Option<(&Dial, ContentFlags)> {
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
    env: Lookup<'_>,
    dial: &Dial,
    have: ContentFlags,
    map: String,
    mode: String,
    requires: ContentFlags,
) -> Result<Plan, BoxError> {
    Ok(Plan::Host(HostPlan {
        dial: dial.clone(),
        start_on_load: false,
        room_name: text(env, HOST_NAME).unwrap_or_else(|| MENU_ROOM_NAME.to_owned()),
        map,
        mode,
        room_size: room_size_from(env)?,
        requires,
        have,
    }))
}

/// A browser row the player chose to join.
pub(super) fn menu_join(dial: &Dial, have: ContentFlags, room: AdvertId, map: String, mode: String) -> Plan {
    Plan::Join(JoinPlan { dial: dial.clone(), room, map, mode, have })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const ROOM: &str = "00112233445566778899aabbccddeeff";

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |name| map.get(name).map(OsString::from)
    }

    fn launch(pairs: &[(&str, &str)]) -> Result<Plan, BoxError> {
        launch_plan(&env_of(pairs), "mp_x", ContentFlags(8), ContentFlags(8))
    }

    const DIAL: [(&str, &str); 2] = [(ADDRESS, "lobby:4433"), (SERVER_NAME, "lobby")];

    #[test]
    fn no_address_means_off() {
        assert_eq!(launch(&[]).unwrap(), Plan::Off);
        assert_eq!(browse_plan(&env_of(&[]), ContentFlags(8)).unwrap(), Plan::Off);
    }

    #[test]
    fn address_without_server_name_is_refused() {
        let error = launch(&[(ADDRESS, "lobby:4433")]).unwrap_err();
        assert_eq!(error.to_string(), "BO2MP_MASTER_ADDR requires BO2MP_MASTER_SERVER_NAME");
        let error = browse_plan(&env_of(&[(ADDRESS, "a")]), ContentFlags(0)).unwrap_err();
        assert_eq!(error.to_string(), "BO2MP_MASTER_ADDR requires BO2MP_MASTER_SERVER_NAME");
    }

    #[test]
    fn browse_keeps_the_ca_file() {
        let env = env_of(&[DIAL[0], DIAL[1], (CA_FILE, "roots.pem")]);
        let Plan::Browse { dial, have } = browse_plan(&env, ContentFlags(4)).unwrap() else {
            panic!("expected a browse plan");
        };
        assert_eq!(dial.address, "lobby:4433");
        assert_eq!(dial.server_name, "lobby");
        assert_eq!(dial.ca_file, Some(PathBuf::from("roots.pem")));
        assert_eq!(have, ContentFlags(4));
    }

    #[test]
    fn host_defaults_to_dm_and_a_full_room() {
        let Plan::Host(host) = launch(&[DIAL[0], DIAL[1], (HOST_NAME, "my room")]).unwrap() else {
            panic!("expected a host plan");
        };
        assert!(host.start_on_load);
        assert_eq!(host.room_name, "my room");
        assert_eq!(host.map, "mp_x");
        assert_eq!(host.mode, "dm");
        assert_eq!(host.room_size, MAX_SESSION_MEMBERS);
        assert_eq!(host.dial.ca_file, None);
    }

    #[test]
    fn host_reads_mode_and_room_size() {
        let pairs = [DIAL[0], DIAL[1], (HOST_NAME, "r"), (GAME_MODE, "tdm"), (ROOM_SIZE, "6")];
        let Plan::Host(host) = launch(&pairs).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!(host.mode, "tdm");
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
        assert_eq!(join.room, ROOM.parse::<AdvertId>().unwrap());
        assert_eq!(join.mode, "dm");
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
        let dial = Dial { address: "a:1".into(), server_name: "a".into(), ca_file: None };
        let plain = menu_host(&env_of(&[]), &dial, ContentFlags(4), "mp_y".into(), "tdm".into(), ContentFlags(8));
        let Plan::Host(host) = plain.unwrap() else {
            panic!("expected a host plan");
        };
        assert!(!host.start_on_load);
        assert_eq!(host.room_name, MENU_ROOM_NAME);
        assert_eq!((host.map.as_str(), host.mode.as_str()), ("mp_y", "tdm"));
        assert_eq!((host.have, host.requires), (ContentFlags(4), ContentFlags(8)));
        assert_eq!(host.room_size, MAX_SESSION_MEMBERS);
        let named = env_of(&[(HOST_NAME, ""), (ROOM_SIZE, "4")]);
        let Plan::Host(host) = menu_host(&named, &dial, ContentFlags(0), "m".into(), "dm".into(), ContentFlags(0)).unwrap() else {
            panic!("expected a host plan");
        };
        assert_eq!((host.room_name.as_str(), host.room_size), ("", 4));
        let too_big = env_of(&[(ROOM_SIZE, "40")]);
        assert!(menu_host(&too_big, &dial, ContentFlags(0), "m".into(), "dm".into(), ContentFlags(0)).is_err());
    }

    #[test]
    fn leaving_a_room_goes_back_to_the_browser() {
        let mut join = MasterLaunchIntent(launch(&[DIAL[0], DIAL[1], (JOIN_ROOM, ROOM)]).unwrap());
        assert!(join.browsing().is_none());
        join.back_to_browser();
        let (dial, have) = join.browsing().expect("browsing again");
        assert_eq!((dial.address.as_str(), have), ("lobby:4433", ContentFlags(8)));
        let room = ROOM.parse::<AdvertId>().unwrap();
        let picked = menu_join(dial, have, room, "mp_z".into(), "dm".into());
        let Plan::Join(plan) = picked else {
            panic!("expected a join plan");
        };
        assert_eq!((plan.room, plan.have, plan.map.as_str()), (room, ContentFlags(8), "mp_z"));
        let mut off = MasterLaunchIntent::disabled();
        off.back_to_browser();
        assert!(!off.enabled());
    }

    #[test]
    fn intent_flags() {
        let off = MasterLaunchIntent::disabled();
        assert!(!off.enabled() && !off.configured() && !off.is_join());
        let join = MasterLaunchIntent(launch(&[DIAL[0], DIAL[1], (JOIN_ROOM, ROOM)]).unwrap());
        assert!(join.enabled() && join.configured() && join.is_join());
        assert_eq!(join.have(), ContentFlags(8));
        assert_eq!(join.dial().map(|dial| dial.server_name.as_str()), Some("lobby"));
    }
}
