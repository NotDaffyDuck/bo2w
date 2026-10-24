//! Guest weapons from Valve's Source games, read from the player's own
//! files: Half-Life 2, Garry's Mod, Half-Life 2: Deathmatch, or the free
//! Garry's Mod Dedicated Server. Nothing of Valve's is in this repository.

pub mod locate;
pub mod mdl;
pub mod portalgun;
pub mod standin;
pub mod vpk;
pub mod vtf;

pub use glam;
pub use locate::Source;

/// Every file the Gravity Gun reads, by its path inside the game.
pub const GRAVITY_GUN: &[&str] = &[
    "models/weapons/v_physcannon.mdl",
    "models/weapons/v_physcannon.vvd",
    "models/weapons/v_physcannon.dx90.vtx",
    "models/weapons/v_superphyscannon.mdl",
    "models/weapons/v_superphyscannon.vvd",
    "models/weapons/v_superphyscannon.dx90.vtx",
    "models/weapons/w_physics.mdl",
    "models/weapons/w_physics.vvd",
    "models/weapons/w_physics.dx90.vtx",
    "materials/models/weapons/v_physcannon/v_physcannon_sheet.vtf",
    "materials/models/weapons/v_physcannon/v_superphyscannon_sheet.vtf",
    "materials/models/weapons/v_hand/v_hand_sheet.vtf",
    "materials/models/weapons/w_physics/w_physics_sheet.vtf",
    "materials/models/weapons/w_physics/w_physics_sheet2.vtf",
    "materials/sprites/physcannon_bluecore1b.vtf",
    "materials/sprites/physcannon_bluecore2b.vtf",
    "materials/sprites/physcannon_blueflare1.vtf",
    "materials/sprites/physcannon_blueglow.vtf",
    "materials/sprites/orangecore1.vtf",
    "materials/sprites/orangecore2.vtf",
    "materials/sprites/orangeflare1.vtf",
    "materials/sprites/glow04.vtf",
    "materials/sprites/blueflare1.vtf",
    "materials/sprites/lgtning.vtf",
    "materials/effects/fluttercore.vtf",
    "sound/weapons/physcannon/hold_loop.wav",
    "sound/weapons/physcannon/superphys_hold_loop.wav",
    "sound/weapons/physcannon/physcannon_pickup.wav",
    "sound/weapons/physcannon/physcannon_drop.wav",
    "sound/weapons/physcannon/physcannon_dryfire.wav",
    "sound/weapons/physcannon/physcannon_tooheavy.wav",
    "sound/weapons/physcannon/physcannon_claws_open.wav",
    "sound/weapons/physcannon/physcannon_claws_close.wav",
    "sound/weapons/physcannon/physcannon_charge.wav",
    "sound/weapons/physcannon/superphys_launch1.wav",
    "sound/weapons/physcannon/superphys_launch2.wav",
    "sound/weapons/physcannon/superphys_launch3.wav",
    "sound/weapons/physcannon/superphys_launch4.wav",
    "sound/weapons/physcannon/superphys_small_zap1.wav",
    "sound/weapons/physcannon/superphys_small_zap2.wav",
    "sound/weapons/physcannon/superphys_small_zap3.wav",
    "sound/weapons/physcannon/superphys_small_zap4.wav",
];
