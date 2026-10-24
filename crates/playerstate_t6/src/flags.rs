//! BO2's player flag bits, as BO2's own movement, weapon and script code
//! sets and tests them. Bits with no setter or reader in BO2's code are
//! left out.

/// `playerState_s::pm_flags`.
pub mod pm_flags {
    /// The stance bits (`PM_CheckDuck`): 0 stand, `PRONE` or `CROUCH`.
    pub const STANCE_MASK: i32 = 0x3;
    pub const PRONE: i32 = 0x1;
    pub const CROUCH: i32 = 0x2;
    /// Mantling (`Mantle_Start` sets it, `Mantle_Move` clears it): blocks
    /// aim (`PM_IsAdsAllowed`).
    pub const MANTLE: i32 = 0x4;
    /// On a ladder (`PM_CheckLadderMove`; `PM_ClearLadderFlag` clears it).
    pub const LADDER: i32 = 0x8;
    /// Aiming down sights (`PM_UpdateAimDownSightFlag`).
    pub const ADS: i32 = 0x10;
    /// Moving backwards: `PmoveSingle` sets it on negative forwardmove.
    pub const BACKWARDS_MOVEMENT: i32 = 0x20;
    /// Walking (`PM_UpdatePlayerWalkingFlag`).
    pub const WALKING: i32 = 0x40;
    /// Hard-landing slowdown, timed by `pm_time` (`PM_CrashLand`).
    pub const TIME_LAND: i32 = 0x80;
    /// Knockback, timed by `pm_time` (`PlayerCmd_ApplyKnockBack`).
    pub const TIME_KNOCKBACK: i32 = 0x100;
    /// Prone movement override (`PM_SetProneMovementOverride`; ADS while
    /// prone sets it with `ADS`).
    pub const PRONE_MOVE_OVERRIDE: i32 = 0x200;
    /// Just spawned: no weapon input until the first raise
    /// (`ClientSpawn` sets it, `PM_Weapon` returns early while set).
    pub const INITIAL_RAISE: i32 = 0x400;
    /// Controls frozen (`ClientEndFrame` copies it from the client).
    pub const FROZEN: i32 = 0x800;
    /// A stance change was refused: the HUD flashes the stance icon
    /// (`PM_CheckDuck`, `CG_DrawPlayerStance`).
    pub const STANCE_BLOCKED: i32 = 0x1000;
    /// Just fell off a ladder (`PM_ClearLadderFlag` sets it).
    pub const LADDER_FALL: i32 = 0x2000;
    /// Jump slowdown, timed by `pm_time` (`Jump_ActivateSlowdown`).
    pub const TIME_JUMP: i32 = 0x4000;
    /// Sprinting (`PM_StartSprint`, `PM_EndSprint`).
    pub const SPRINT: i32 = 0x8000;
    /// Shell shocked (`GScr_ShellShock`).
    pub const SHELLSHOCK: i32 = 0x1_0000;
    /// Knife lunge running (`PM_MeleeChargeStart`): forwardmove is held at
    /// 127.
    pub const MELEE_CHARGE: i32 = 0x2_0000;
    /// The `AllowX(0)` script calls set these.
    pub const NO_SPRINT: i32 = 0x4_0000;
    pub const NO_JUMP: i32 = 0x8_0000;
    /// A scripted animation is playing (`PM_UpdateScriptedAnim`).
    pub const SCRIPTED_ANIM: i32 = 0x10_0000;
    /// Dive to prone (`Dtp_Start` sets it, `Dtp_CheckForEnd` clears it).
    pub const DTP: i32 = 0x20_0000;
    /// Sliding (`PM_GroundTrace`; `GScr_IsOnSlide` reads it).
    pub const SLIDE: i32 = 0x40_0000;
    pub const NO_STAND: i32 = 0x80_0000;
    pub const NO_CROUCH: i32 = 0x100_0000;
    pub const NO_PRONE: i32 = 0x200_0000;
    /// `PM_CheckDuck`'s mask of the three stance bans.
    pub const STANCE_BANS: i32 = NO_STAND | NO_CROUCH | NO_PRONE;
    /// Flipped each footstep, left foot then right (`PM_FootstepEvent`).
    pub const FOOTSTEP_ALT: i32 = 0x400_0000;
    pub const NO_LEAN: i32 = 0x1000_0000;
    pub const NO_MELEE: i32 = 0x2000_0000;
    /// Every surface is slick (`PlayerCmd_ForceSlick`).
    pub const FORCE_SLICK: i32 = 0x4000_0000;
}

/// `playerState_s::e_flags`, shared with the entity's `eFlags` through
/// `BG_PlayerStateToEntityState`.
pub mod e_flags {
    pub const CROUCHING: i32 = 0x4;
    pub const PRONE: i32 = 0x8;
    /// Firing this frame (`PmoveSingle`).
    pub const FIRING: i32 = 0x40;
    /// No user command for over a second (`ClientEndFrame`, entity only).
    pub const CONNECTION_INTERRUPTED: i32 = 0x80;
    /// On a mounted turret: `turret_use` sets one half or both by stance.
    pub const TURRET_STAND: i32 = 0x100;
    pub const TURRET_CROUCH: i32 = 0x200;
    pub const TURRET_MASK: i32 = TURRET_STAND | TURRET_CROUCH;
    /// Aim assist may lock onto this entity (`GScr_EnableAimAssist`).
    pub const AIM_ASSIST: i32 = 0x800;
    /// Riding a vehicle (`VEH_LinkPlayer` sets it, `VEH_PlayerExit` clears
    /// it).
    pub const IN_VEHICLE: i32 = 0x4000;
    /// Mantling (`Mantle_Start`).
    pub const MANTLE: i32 = 0x8000;
    /// In a vehicle's gunner seat (`VEH_UpdateGunnerWeapon`).
    pub const VEHICLE_GUNNER: i32 = 0x1_0000;
    /// Dead: `BG_PlayerStateToEntityState` sets it when `pm_type` is dead.
    pub const DEAD: i32 = 0x4_0000;
    /// Aiming down sights, copied from `pm_flags::ADS`.
    pub const ADS: i32 = 0x8_0000;
    pub const VOTED: i32 = 0x10_0000;
    /// Shows on the enemy's minimap (`PlayerCmd_pingPlayer`).
    pub const PING: i32 = 0x40_0000;
}

/// `playerState_s::e_flags2`.
pub mod e_flags2 {
    pub const DUAL_GRENADES: i32 = 0x100;
    pub const SPAWN_PROTECTION: i32 = 0x200;
    /// Fired since the last reload (`PM_Weapon_FireWeapon` sets it,
    /// `PM_ReloadClip` clears it).
    pub const FIRED_SINCE_RELOAD: i32 = 0x400;
    pub const STOWED_WEAPON: i32 = 0x800;
    pub const GPS_JAMMER: i32 = 0x1000;
    pub const GUIDED_MISSILE_CAMERA: i32 = 0x4_0000;
    /// Set with `other_flags::EMP_JAMMED` (`GScr_SetEMPJammed`).
    pub const EMP_JAMMED: i32 = 0x8_0000;
    pub const BURNING: i32 = 0x20_0000;
    pub const TURRET_SPINNING: i32 = 0x40_0000;
    /// Driving a remote-controlled vehicle or missile (`VEH_LinkPlayer`).
    pub const REMOTE_CONTROL: i32 = 0x1000_0000;
    pub const DRAW_INFRARED: i32 = 0x2000_0000;
}

/// `playerState_s::other_flags`.
pub mod other_flags {
    pub const INVULNERABLE: i32 = 0x1;
    pub const SPECTATING: i32 = 0x2;
    /// In the game, not spectating (`ClientEndFrame`): `PM_IsAdsAllowed`
    /// needs it.
    pub const PLAYING: i32 = 0x4;
    pub const CAN_FOLLOW: i32 = 0x8;
    pub const CAN_FREE_FLY: i32 = 0x10;
    /// Spectating or following someone (`GetFollowing`, `CG_DrawFollow`).
    pub const SPECTATOR_MASK: i32 = SPECTATING | CAN_FOLLOW | CAN_FREE_FLY;
    pub const EMP_JAMMED: i32 = 0x40;
    pub const RIOTSHIELD_ATTACHED: i32 = 0x80;
    /// No weapon or look input (`ClientEndFrame` copies it from the
    /// client).
    pub const FROZEN: i32 = 0x400;
    pub const SERVER_VISIONSET: i32 = 0x800;
    pub const INFRARED: i32 = 0x1000;
    pub const CAMERA_LINKED: i32 = 0x2000;
}

/// `playerState_s::weap_flags` (64 bits; BO2 only uses the low 32).
pub mod weap_flags {
    /// An offhand is held for use (`PM_Weapon_OffHand_HoldUse`).
    pub const OFFHAND_HOLDUSE: u64 = 0x2;
    pub const HOLD_BREATH: u64 = 0x4;
    pub const AIM_AT_FRIENDLY: u64 = 0x8;
    pub const AIM_AT_ENEMY: u64 = 0x10;
    /// `PlayerCmd_AllowADS(0)`: blocks aim (`PM_IsAdsAllowed`).
    pub const NO_ADS: u64 = 0x20;
    pub const NIGHTVISION: u64 = 0x40;
    pub const WEAPONS_DISABLED: u64 = 0x80;
    pub const OFFHAND_DISABLED: u64 = 0x100;
    pub const CYCLING_DISABLED: u64 = 0x200;
    pub const OFFHAND_INIT: u64 = 0x4_0000;
    pub const OFFHAND_ENDED: u64 = 0x10_0000;
    /// Fired since the weapon was raised (`PM_Weapon_StartFiring`).
    pub const FIRED_SINCE_RAISE: u64 = 0x20_0000;
    pub const CAMERA_SPIKE: u64 = 0x80_0000;
    pub const CARRYING_TURRET: u64 = 0x100_0000;
    pub const CHARGING_SHOT: u64 = 0x400_0000;
}

/// `playerState_s::link_flags`.
pub mod link_flags {
    pub const OFFSET: i32 = 0x2;
    /// Linked to `tag_player`: the player may still aim
    /// (`PlayerLinkTo_Internal`, `PM_IsAdsAllowed`).
    pub const TAG_PLAYER: i32 = 0x4;
    /// Linked by blend, origin fixed (`ScrCmd_PlayerLinkToBlend`).
    pub const BLEND: i32 = 0x8;
}

/// `playerState_s::move_type` when sliding (`BG_IsSliding`).
pub const MOVE_TYPE_SLIDE: i32 = 0x1a;

/// The `view_height_target` values `PM_GetEffectiveStance` tells apart.
pub mod view_height {
    pub const PRONE: i32 = 0xb;
    /// 22 units: no stance name in BO2's code. It reads as prone on dry
    /// land, crouch in shallow water, stand in deep water.
    pub const LOW: i32 = 0x16;
    pub const CROUCH: i32 = 0x28;
}

/// BO2's `PmStanceFrontBack`, what `PM_GetEffectiveStance` returns.
pub mod stance {
    pub const STAND: i32 = 0;
    pub const PRONE: i32 = 1;
    pub const CROUCH: i32 = 2;
    pub const BACKWARD_RUN: i32 = 3;
    pub const BACKWARD_PRONE: i32 = 4;
    pub const BACKWARD_CROUCH: i32 = 5;
}
