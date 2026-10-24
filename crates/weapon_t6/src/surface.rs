//! BO2's surface type names, indexed by the surface type a trace's `sflags` carry.

/// How many surface types BO2 names.
pub const SURFACE_TYPE_COUNT: usize = 32;

/// Black Ops II's surface types, in its order (its `bullet_penetration_mp`
/// keys and its maps' surface flags name these): the older games' list up
/// to `paintedmetal`, then `player`, `tallgrass` and `riotshield`.
pub const T6_SURFACE_TYPE_NAMES: [&str; SURFACE_TYPE_COUNT] = [
    "default",
    "bark",
    "brick",
    "carpet",
    "cloth",
    "concrete",
    "dirt",
    "flesh",
    "foliage",
    "glass",
    "grass",
    "gravel",
    "ice",
    "metal",
    "mud",
    "paper",
    "plaster",
    "rock",
    "sand",
    "snow",
    "water",
    "wood",
    "asphalt",
    "ceramic",
    "plastic",
    "rubber",
    "cushion",
    "fruit",
    "paintedmetal",
    "player",
    "tallgrass",
    "riotshield",
];
