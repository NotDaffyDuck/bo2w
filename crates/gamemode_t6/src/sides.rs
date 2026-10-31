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

    /// Every team, positioned at its own discriminant.
    const IN_NUMBER_ORDER: [Team; Team::COUNT as usize] = [
        Team::Free,
        Team::Allies,
        Team::Axis,
        Team::Three,
        Team::Four,
        Team::Five,
        Team::Six,
        Team::Seven,
        Team::Eight,
        Team::Spectator,
    ];

    /// The gsc names, positioned at each team's discriminant.
    const SCRIPT_NAMES: [&'static str; Team::COUNT as usize] = [
        "free",
        "allies",
        "axis",
        "team3",
        "team4",
        "team5",
        "team6",
        "team7",
        "team8",
        "spectator",
    ];

    pub fn from_packed_u8(raw: u8) -> Option<Self> {
        Self::IN_NUMBER_ORDER.get(usize::from(raw)).copied()
    }

    /// The other side of an allies-versus-axis fight; every other team has no rival and maps to itself.
    pub fn rival_side(self) -> Self {
        let (first, second) = (Team::Allies, Team::Axis);
        if self == first {
            second
        } else if self == second {
            first
        } else {
            self
        }
    }

    /// The gsc team string: "allies", "axis", "free", "spectator", "team3".."team8".
    pub fn script_name(self) -> &'static str {
        Self::SCRIPT_NAMES[self as usize]
    }
}

// gm-5: g_TeamColor_MyTeam / g_TeamColor_EnemyTeam exist in BO2; their defaults are not in any
// dump. TODO(fact): the BO2 default values.
pub const FRIENDLY_TINT: &str = ".6 .8 .6";

pub const HOSTILE_TINT: &str = "1 .45 .5";
