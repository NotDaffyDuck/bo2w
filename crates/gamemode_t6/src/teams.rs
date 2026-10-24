/// BO2 `team_t` (gm-1). The numbers are BO2's: allies 1, axis 2 (legacy had them the other way
/// round), so convert other crates' team numbers by name, never by value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Team {
    #[default]
    Free = 0,
    Allies = 1,
    Axis = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Spectator = 9,
}

impl Team {
    /// gm-1: NUM_TEAMS.
    pub const COUNT: u8 = 10;

    pub fn from_packed_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Free),
            1 => Some(Self::Allies),
            2 => Some(Self::Axis),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            6 => Some(Self::Six),
            7 => Some(Self::Seven),
            8 => Some(Self::Eight),
            9 => Some(Self::Spectator),
            _ => None,
        }
    }

    pub fn opposing(self) -> Self {
        match self {
            Self::Allies => Self::Axis,
            Self::Axis => Self::Allies,
            other => other,
        }
    }

    /// The gsc team string: "allies", "axis", "free", "spectator", "team3".."team8".
    pub fn script_name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Allies => "allies",
            Self::Axis => "axis",
            Self::Three => "team3",
            Self::Four => "team4",
            Self::Five => "team5",
            Self::Six => "team6",
            Self::Seven => "team7",
            Self::Eight => "team8",
            Self::Spectator => "spectator",
        }
    }
}

// gm-5: g_TeamColor_MyTeam / g_TeamColor_EnemyTeam exist in BO2; their defaults are not in any
// dump. TODO(fact): the BO2 default values.
pub const TEAM_COLOR_MY_TEAM: &str = ".6 .8 .6";

pub const TEAM_COLOR_ENEMY_TEAM: &str = "1 .45 .5";
