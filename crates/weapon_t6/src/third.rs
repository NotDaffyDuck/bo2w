//! Window 14: other players' gun anims (third person), BO2's way.
//!
//! What another player (or a bot) looks like while he swaps, reloads, fires, throws, knifes,
//! lunges, sprints, dives and slides: which body anim plays on his legs and torso, when it may
//! start, and how long it holds. Spec: `docs/gun/THIRD.md`.
//!
//! - [`AnimEvent`] / [`AnimCall`]: the anim events the gun and movement code fire at BO2's exact
//!   points, each with BO2's two flags (continue, force).
//! - [`Conds`]: the conditions the anim script blocks test, worked out the way BO2 does.
//! - [`MoveIn`] / [`move_type`]: the legs' movement type each frame (BO2's stance x speed x
//!   stumble table, then sprint, dive, slide, swim).
//! - [`Body`]: the two anim slots and their timers, with BO2's start rules ([`Body::play`]).
//! - [`script_event`] / [`script_move`] / [`apply_movement`]: look up the script through an
//!   [`AnimTable`] and start the result. The table (the parsed `playeranim.script`) lives with
//!   whoever loads it (sim's `T6PlayerAnims`); this file only needs [`AnimTable`].
//!
//! The numbers below are BO2's defaults, read from the BO2 server program's dvar registration and
//! data tables.

use crate::state;

/// BO2 dvar defaults this file uses.
pub mod dvars {
    /// `bg_maxWeaponAnimScale`: a weapon-timed anim plays at most this much faster or slower than
    /// its own speed.
    pub const MAX_WEAPON_ANIM_SCALE: f32 = 0.66;
    /// `bg_playerAnimStanceAllowMovementInteruptTime` (ms): a stance-change anim can be cut by a
    /// moving anim once it has played this long.
    pub const STANCE_MOVE_INTERRUPT_MS: i32 = 200;
    /// `bg_forceAnimOverrideTimerCheck`.
    pub const FORCE_TIMER_CHECK: bool = false;
    /// `player_moveThreshhold` (units/s).
    pub const MOVE: f32 = 2.0;
    /// `player_runThreshhold` (units/s).
    pub const RUN: f32 = 110.0;
    /// `player_knockbackMoveThreshhold` (units/s).
    pub const KNOCKBACK_MOVE: f32 = 160.0;
    /// `player_animWalkThreshhold` (units/s).
    pub const ANIM_WALK: f32 = 30.0;
    /// `player_animRunThreshhold` (units/s).
    pub const ANIM_RUN: f32 = 140.0;
    /// `player_sprintThreshhold` (units/s).
    pub const SPRINT: f32 = 185.0;
    /// `player_enableShuffleAnims`.
    pub const SHUFFLE_ANIMS: bool = false;
    /// `player_useSlopeAnims`.
    pub const SLOPE_ANIMS: bool = true;
    /// `player_slopeAnimAngle` (degrees).
    pub const SLOPE_ANGLE: f32 = 30.0;
    /// `player_dmgtimer_stumbleTime` (ms).
    pub const STUMBLE_MS: i32 = 500;
}

/// A part that is still busy longer than this (ms) keeps its anim unless forced.
pub const BUSY_MS: i32 = 50;

/// The restart bit in a legs / torso anim value: flips each time the anim starts over.
pub const RESTART_BIT: u16 = 0x400;

/// Anim value without the restart bit.
pub const fn clip_ordinal(value: u16) -> u16 {
    value & !RESTART_BIT
}

/// Flags BO2 marks on each script anim when the script loads.
pub mod anim_flags {
    /// Fire anims (they also get a 30 ms blend-in).
    pub const TRIGGER_SLOT: u32 = 0x8;
    /// Death anims (no move speed).
    pub const DEATH: u32 = 0x100;
    /// A looping anim: a `continue` call refreshes its timer.
    pub const LOOPED: u32 = 0x200;
    /// Knife and knife-lunge anims.
    pub const KNIFE: u32 = 0x400;
    /// Script-event anims.
    pub const SCRIPT_EVENT: u32 = 0x800;
    /// The command plays for the anim's own length, not a `duration` it gave.
    pub const OWN_LENGTH: u32 = 0x1000;
    /// `weaponTimeScale`: plays over the gun's current action time.
    pub const WEAPON_TIME_SCALE: u32 = 0x4000;
    /// `weaponDelayScale`: plays over the gun's current delay.
    pub const WEAPON_DELAY_SCALE: u32 = 0x8000;
    /// A walk anim (others read "walking" from it).
    pub const WALK: u32 = 0x200_0000;
    /// A run anim (others read "moving" from it).
    pub const RUN: u32 = 0x400_0000;
    /// A stance-change anim: a moving anim may cut it.
    pub const STANCE: u32 = 0x800_0000;
    /// A reload anim: a drop / raise may cut it.
    pub const RELOAD: u32 = 0x1000_0000;
    /// Drop / raise anims (and `allowReloadInterrupt`): they cut a reload.
    pub const CUTS_RELOAD: u32 = 0x2000_0000;
}

/// BO2's anim events, by BO2's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AnimEvent {
    Death = 1,
    FireWeapon = 2,
    Jump = 3,
    Land = 4,
    DropWeapon = 5,
    RaiseWeapon = 6,
    FirstRaiseWeapon = 7,
    Reload = 8,
    CrouchToProne = 9,
    ProneToCrouch = 10,
    StandToCrouch = 11,
    CrouchToStand = 12,
    ProneToStand = 13,
    ProneToSprint = 14,
    RunToIdle = 15,
    MeleeAttack = 16,
    MeleeLeft = 17,
    KnifeMelee = 18,
    KnifeMeleeCharge = 19,
    Flinch = 20,
    Shellshock = 21,
    VehicleEnter = 22,
    VehicleChangeSeats = 23,
    VehicleExit = 24,
    ScriptEvent = 25,
    DtpTakeoff = 26,
    DtpLand = 27,
    StandToLaststand = 28,
    CrouchToLaststand = 29,
    ProneToLaststand = 30,
    LaststandToStand = 31,
    LaststandToCrouch = 32,
    LaststandToProne = 33,
    Flared = 34,
    LaststandSuicide = 35,
    PrimeGrenade = 36,
}

impl AnimEvent {
    /// The event's name in `playeranim.script` (lower case).
    pub const fn script_name(self) -> &'static str {
        use AnimEvent::*;
        match self {
            Death => "death",
            FireWeapon => "fireweapon",
            Jump => "jump",
            Land => "land",
            DropWeapon => "dropweapon",
            RaiseWeapon => "raiseweapon",
            FirstRaiseWeapon => "firstraiseweapon",
            Reload => "reload",
            CrouchToProne => "crouch_to_prone",
            ProneToCrouch => "prone_to_crouch",
            StandToCrouch => "stand_to_crouch",
            CrouchToStand => "crouch_to_stand",
            ProneToStand => "prone_to_stand",
            ProneToSprint => "prone_to_sprint",
            RunToIdle => "run_to_idle",
            MeleeAttack => "meleeattack",
            MeleeLeft => "meleeleft",
            KnifeMelee => "knife_melee",
            KnifeMeleeCharge => "knife_melee_charge",
            Flinch => "flinch",
            Shellshock => "shellshock",
            VehicleEnter => "vehicle_enter",
            VehicleChangeSeats => "vehicle_change_seats",
            VehicleExit => "vehicle_exit",
            ScriptEvent => "scriptevent",
            DtpTakeoff => "dtp_takeoff",
            DtpLand => "dtp_land",
            StandToLaststand => "stand_to_laststand",
            CrouchToLaststand => "crouch_to_laststand",
            ProneToLaststand => "prone_to_laststand",
            LaststandToStand => "laststand_to_stand",
            LaststandToCrouch => "laststand_to_crouch",
            LaststandToProne => "laststand_to_prone",
            Flared => "flared",
            LaststandSuicide => "laststand_suicide",
            PrimeGrenade => "prime_grenade",
        }
    }
}

/// One anim event as the code fires it: `cont` = don't restart the same anim, `force` = start
/// even while the part is busy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimCall {
    pub event: AnimEvent,
    pub cont: bool,
    pub force: bool,
}

impl AnimCall {
    pub const fn new(event: AnimEvent, cont: bool, force: bool) -> Self {
        Self { event, cont, force }
    }

    /// A shot starts, a riot shield is planted, or an offhand is let go (StartFiring,
    /// RiotshieldStart, OffHandStart).
    pub const TRIGGER_SLOT: Self = Self::new(AnimEvent::FireWeapon, false, true);
    /// A weapon switch starts.
    pub const DROP: Self = Self::new(AnimEvent::DropWeapon, false, false);
    /// A held offhand is quick-dropped (OffHand_HoldUse).
    pub const DROP_OFFHAND: Self = Self::new(AnimEvent::DropWeapon, false, true);
    /// A reload starts (BeginWeaponReload, or a prone change mid-reload).
    pub const RELOAD: Self = Self::new(AnimEvent::Reload, false, true);
    /// A dive to prone takes off.
    pub const DTP_TAKEOFF: Self = Self::new(AnimEvent::DtpTakeoff, false, true);
    /// A dive to prone lands.
    pub const DTP_LAND: Self = Self::new(AnimEvent::DtpLand, true, true);
    /// He stops from a run with no move keys held.
    pub const RUN_TO_IDLE: Self = Self::new(AnimEvent::RunToIdle, false, false);

    /// A switch finishes: raise, or first raise for a weapon he never had before.
    pub fn raise(first: bool) -> Self {
        let event = if first {
            AnimEvent::FirstRaiseWeapon
        } else {
            AnimEvent::RaiseWeapon
        };
        Self::new(event, false, false)
    }

    /// A melee starts: see [`melee_event`].
    pub fn melee(event: AnimEvent) -> Self {
        Self::new(event, false, true)
    }

    /// An offhand is primed (OffHandPrepare): prime_grenade, or raiseweapon for gear.
    pub fn prime(gear: bool) -> Self {
        let event = if gear {
            AnimEvent::RaiseWeapon
        } else {
            AnimEvent::PrimeGrenade
        };
        Self::new(event, true, true)
    }

    /// The call for each anim event the gun code sends ([`crate::events::AnimScript`]), with BO2's
    /// flags for that call site.
    pub fn from_script(script: crate::events::AnimScript) -> Self {
        use crate::events::AnimScript;
        match script {
            AnimScript::DropWeapon => Self::DROP,
            AnimScript::RaiseWeapon => Self::raise(false),
            AnimScript::FirstRaiseWeapon => Self::raise(true),
            AnimScript::PrimeGrenade => Self::prime(false),
            AnimScript::PrimeGear => Self::prime(true),
            AnimScript::Reload => Self::RELOAD,
            AnimScript::FireWeapon | AnimScript::RiotshieldStart | AnimScript::OffHandStart => {
                Self::TRIGGER_SLOT
            }
            AnimScript::DtpTakeoff => Self::DTP_TAKEOFF,
            AnimScript::DtpLand => Self::DTP_LAND,
            AnimScript::RunToIdle => Self::RUN_TO_IDLE,
            AnimScript::MeleeAttack => Self::melee(AnimEvent::MeleeAttack),
            AnimScript::KnifeMelee => Self::melee(AnimEvent::KnifeMelee),
            AnimScript::MeleeLeft => Self::melee(AnimEvent::MeleeLeft),
            AnimScript::KnifeMeleeCharge => Self::melee(AnimEvent::KnifeMeleeCharge),
        }
    }
}

/// Which melee event a swing fires (MeleeInit's choice), checked in BO2's order.
pub fn melee_event(riotshield: bool, lunge: bool, left_hand: bool, blade_mesh: bool) -> AnimEvent {
    if riotshield {
        AnimEvent::MeleeAttack
    } else if lunge && blade_mesh {
        AnimEvent::KnifeMeleeCharge
    } else if left_hand {
        AnimEvent::MeleeLeft
    } else if blade_mesh {
        AnimEvent::KnifeMelee
    } else {
        AnimEvent::MeleeAttack
    }
}

/// A lunge: the charge flag is set and the target is beyond plain melee range.
pub fn is_lunge(charge_flag: bool, charge_dist: f32, melee_range: f32) -> bool {
    charge_flag && charge_dist > melee_range
}

/// Dual wield shows a reload on the body only when both guns are reloading together and both
/// still have more than half their reload to go.
pub fn dual_reload_shows(
    state_right: i32,
    state_left: i32,
    time_right: i32,
    countdown_rest: i32,
    reload_right: i32,
    reload_left: i32,
) -> bool {
    state_right == state::RELOADING
        && state_left == state::RELOADING
        && time_right > reload_right >> 1
        && countdown_rest > reload_left >> 1
}

/// [`dual_reload_shows`] on the player state: the held (right) gun and its left partner, each
/// hand's state and timer, and each gun's own reload time. False when the held gun is not dual.
pub fn dual_reload_shows_ps(
    wps: &crate::player::T6WeaponPs,
    facts: &crate::facts::FactsTable,
) -> bool {
    use crate::pm::dual;
    if !dual::is_dual_wield(facts, wps.weapon) {
        return false;
    }
    let left = dual::dual_wield_weapon(facts, wps.weapon);
    dual_reload_shows(
        wps.gun_phase,
        wps.weaponstate_left,
        wps.gun_timer_ms,
        wps.weapon_time_left,
        facts.get(wps.weapon).reload.reload,
        facts.get(left).reload.reload,
    )
}

/// First raise when he never had this weapon, except in pm types 7 and 9.
pub fn first_raise(ever_had: bool, movement_mode: i32) -> bool {
    !ever_had && movement_mode != 7 && movement_mode != 9
}

/// The conditions the script blocks test, by BO2's condition numbers (section 2 of the spec).
/// Values are indexes into each condition's word list in the script.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conds {
    /// 0: the weapon's `playerAnimType` (index in playeranimtypes.txt).
    pub anim_type: u8,
    /// 1: the weapon's class.
    pub weapon_class: u8,
    /// 2: the anim type of the weapon he is switching to.
    pub next_anim_type: u8,
    /// 3: the class of the weapon he is switching to.
    pub next_weapon_class: u8,
    /// 4: on a turret.
    pub mounted: u8,
    /// 5: 0 still, 1 walking, 2 moving ([`move_status`]).
    pub move_status: u8,
    /// 6: last move direction.
    pub prev_direction: u8,
    /// 7: move direction.
    pub direction: u8,
    /// 10: 0 stand, 1 crouch, 2 prone.
    pub stance: u8,
    /// 11: looking down.
    pub underhand: bool,
    /// 12: attack held.
    pub firing: bool,
    /// 13: [`weapon_position`].
    pub weapon_position: u8,
    /// 14: [`slope`].
    pub slope: u8,
    /// 15: perk bits.
    pub perk: u8,
    /// 16: the weapon has a bayonet.
    pub attachment: bool,
    /// 17: switching to the riot shield.
    pub riotshield_next: bool,
}

/// Condition 5 for the player himself: over the run threshold = moving (2), over the move
/// threshold = walking (1), else still (0). A knocked-back player slower than the knockback
/// threshold counts as still.
pub fn move_status(xy_speed: f32, knocked_back: bool) -> u8 {
    if knocked_back && xy_speed < dvars::KNOCKBACK_MOVE {
        0
    } else if xy_speed > dvars::RUN {
        2
    } else if xy_speed > dvars::MOVE {
        1
    } else {
        0
    }
}

/// Condition 5 for another player: read from his legs anim's run / walk marks.
pub fn move_status_of_anim(legs_flags: u32) -> u8 {
    if legs_flags & anim_flags::RUN != 0 {
        2
    } else if legs_flags & anim_flags::WALK != 0 {
        1
    } else {
        0
    }
}

/// Condition 13: 1 aiming down the sights, 2 holding fire to place the riot shield, else 0.
/// Others see only 1 (from the entity's ADS flag) or 0.
pub fn weapon_position(ads: bool, weaponstate: i32) -> u8 {
    if ads {
        1
    } else if weaponstate == state::RIOTSHIELD_HOLD {
        2
    } else {
        0
    }
}

/// Condition 14 from the average ground pitch (degrees): 1 uphill, 2 downhill, else 0.
pub fn slope(average_pitch: f32) -> u8 {
    if !dvars::SLOPE_ANIMS {
        0
    } else if average_pitch > dvars::SLOPE_ANGLE {
        1
    } else if average_pitch < -dvars::SLOPE_ANGLE {
        2
    } else {
        0
    }
}

/// Condition 11: looking down (view pitch above 0).
pub fn underhand(view_pitch: f32) -> bool {
    view_pitch > 0.0
}

/// BO2's movement types, by BO2's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MoveType {
    Idle = 1,
    Shuffle = 2,
    Walk = 3,
    Run = 4,
    ClimbUp = 5,
    ClimbDown = 6,
    Sprint = 7,
    TurnRight = 19,
    TurnLeft = 20,
    Stumble = 21,
    StumbleWalk = 22,
    StumbleSprint = 23,
    Swim = 24,
    Dtp = 25,
    Slide = 26,
}

impl MoveType {
    /// The movement type's name in `playeranim.script` (lower case).
    pub const fn script_name(self) -> &'static str {
        use MoveType::*;
        match self {
            Idle => "idle",
            Shuffle => "shuffle",
            Walk => "walk",
            Run => "run",
            ClimbUp => "climbup",
            ClimbDown => "climbdown",
            Sprint => "sprint",
            TurnRight => "turnright",
            TurnLeft => "turnleft",
            Stumble => "stumble",
            StumbleWalk => "stumble_walk",
            StumbleSprint => "stumble_sprint",
            Swim => "swim",
            Dtp => "dtp",
            Slide => "slide",
        }
    }
}

/// BO2's move table: [stance and facing][speed][hurt]. Stance and facing: stand, prone, crouch,
/// then the same three moving backward. Speed: run, walk, shuffle.
const MOVE_TABLE: [[[MoveType; 2]; 3]; 6] = {
    use MoveType::*;
    let stand = [[Run, Stumble], [Walk, StumbleWalk], [Shuffle, Shuffle]];
    let prone = [[Walk, Walk], [Walk, Walk], [Shuffle, Shuffle]];
    let crouch = [[Run, Stumble], [Walk, Stumble], [Shuffle, Shuffle]];
    [stand, prone, crouch, stand, prone, crouch]
};

/// Standing still: no turn, turning right, turning left.
pub const STILL_MOVES: [MoveType; 3] = [MoveType::Idle, MoveType::TurnRight, MoveType::TurnLeft];

/// What the legs' movement type is picked from, for a player who is moving.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveIn {
    /// 0 stand, 1 crouch, 2 prone (BO2's effective stance).
    pub stance: u8,
    /// Moving backward.
    pub backward: bool,
    /// Walking (the walk button) or leaning.
    pub walking: bool,
    /// Sprint flag set.
    pub sprinting: bool,
    /// Diving to prone (the dive flag).
    pub diving: bool,
    /// Sliding.
    pub sliding: bool,
    pub water_level: i32,
    /// Hurt within the stumble time.
    pub hurt: bool,
    pub xy_speed: f32,
}

/// The legs' movement type for a moving player, and whether it is forced (sprint forces).
pub fn move_type(m: &MoveIn) -> (MoveType, bool) {
    // BO2's facing index: stand 0, prone 1, crouch 2 (+3 backward).
    let stance = match m.stance {
        1 => 2,
        2 => 1,
        _ => 0,
    };
    let mut speed = usize::from(m.walking);
    let mut sprint = m.sprinting;
    if stance == 0 && !m.diving {
        sprint = m.sprinting && m.xy_speed >= dvars::SPRINT;
        speed = if m.xy_speed < dvars::ANIM_WALK {
            usize::from(dvars::SHUFFLE_ANIMS) + 1
        } else if m.xy_speed < dvars::ANIM_RUN {
            1
        } else {
            0
        };
    }
    let facing = stance + if m.backward { 3 } else { 0 };
    let mut mt = MOVE_TABLE[facing][speed][usize::from(m.hurt)];
    if facing == 0 && sprint {
        mt = if m.hurt {
            MoveType::StumbleSprint
        } else {
            MoveType::Sprint
        };
    }
    if m.diving {
        mt = MoveType::Dtp;
    }
    if m.sliding {
        mt = MoveType::Slide;
    }
    if m.water_level > 3 {
        mt = MoveType::Swim;
    }
    (mt, sprint)
}

/// Hurt recently enough to stumble.
pub fn hurt(hurt_clock: i32, hurt_length: i32) -> bool {
    (hurt_length - dvars::STUMBLE_MS).max(0) < hurt_clock
}

/// He stopped from a run: no move keys, no dive, and still at least at run speed.
pub fn run_to_idle(move_keys: bool, diving: bool, xy_speed: f32) -> bool {
    !move_keys && !diving && xy_speed >= dvars::RUN
}

/// Which part of the body a command plays on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Legs,
    Torso,
    /// The legs anim drives the whole body; the torso slot is cleared to 0.
    Both,
}

/// One script command: up to two anims (each on a part) and how long it plays (ms, the
/// `duration` it gave or the first anim's own length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub parts: [Option<(Part, u16)>; 2],
    pub span_ms: i32,
}

impl Command {
    pub fn one(part: Part, anim: u16, hold_ms: i32) -> Self {
        Self {
            parts: [Some((part, anim)), None],
            span_ms: hold_ms,
        }
    }
}

/// What this file needs to know about one script anim.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimInfo {
    /// Its own length (ms).
    pub len_ms: i32,
    /// [`anim_flags`].
    pub flags: u32,
    /// It has a move speed (a moving anim).
    pub moves: bool,
}

/// The parsed anim script. Conditions are the table's own type (the script tests them by name or
/// by index); [`Conds`] and its helpers give BO2's values.
pub trait AnimTable {
    type Conds: ?Sized;
    /// The first block of this event whose conditions all match: `pick(count)` chooses which of
    /// its commands runs (only called when it has some). `None` = no block or no commands.
    fn event_command(
        &self,
        event: AnimEvent,
        conds: &Self::Conds,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<Command>;
    /// The same for a movement type.
    fn move_command(
        &self,
        mt: MoveType,
        conds: &Self::Conds,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<Command>;
    fn info(&self, anim: u16) -> AnimInfo;
}

/// BO2's random step (the player's hold-rand seed): picks an event's variant.
pub fn bg_rand(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    *seed >> 17
}

/// Play rate of a weapon-timed anim: its length over the gun's time, held within
/// 1 +/- [`dvars::MAX_WEAPON_ANIM_SCALE`].
pub fn weapon_rate(len_ms: i32, weapon_ms: i32) -> f32 {
    let rate = len_ms as f32 / weapon_ms as f32;
    let max = dvars::MAX_WEAPON_ANIM_SCALE;
    if (rate - 1.0).abs() <= max {
        rate
    } else if rate - 1.0 <= 0.0 {
        1.0 - max
    } else {
        1.0 + max
    }
}

/// The two anim slots of one player, as they go out in his snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Body {
    /// Anim index plus [`RESTART_BIT`].
    pub legs_anim: u16,
    /// How long the legs stay busy (ms).
    pub lower_body_clock: i32,
    pub legs_duration: i32,
    pub torso_anim: u16,
    pub upper_body_clock: i32,
    pub torso_duration: i32,
}

/// One play request on a part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub anim: u16,
    /// Plays for this long (ms) at rate 1.
    pub span_ms: i32,
    pub rate: f32,
    /// An event sets the part's timer; a movement anim does not.
    pub arm_countdown: bool,
    pub cont: bool,
    pub force: bool,
}

impl Body {
    /// Run the timers down by one frame.
    pub fn tick(&mut self, msec: i32) {
        for timer in [&mut self.lower_body_clock, &mut self.upper_body_clock] {
            if *timer > 0 {
                *timer = (*timer - msec).max(0);
            }
        }
    }

    /// Start an anim on a part by BO2's rules. Returns the duration when the legs started it.
    pub fn play(&mut self, part: Part, p: &Play, table: &impl AnimTable) -> Option<i32> {
        let rate = if p.rate > 0.0 { p.rate } else { 1.0 };
        let duration = (p.span_ms as f32 / rate) as i32;
        let new = table.info(p.anim);
        let mut legs_started = false;
        let mut torso_anim = p.anim;
        if matches!(part, Part::Legs | Part::Both) {
            let mut force = p.force;
            let cur = clip_ordinal(self.legs_anim);
            if !force && cur != p.anim && (!dvars::FORCE_TIMER_CHECK || self.lower_body_clock < BUSY_MS) {
                // A stance anim gives way to a moving anim once it has played long enough.
                if table.info(cur).flags & anim_flags::STANCE != 0 && new.moves {
                    let wait = dvars::STANCE_MOVE_INTERRUPT_MS;
                    if wait == 0 || self.lower_body_clock < (self.legs_duration - wait).max(BUSY_MS) {
                        force = true;
                    }
                }
            }
            if self.lower_body_clock < BUSY_MS || force {
                if !p.cont || cur != p.anim {
                    self.legs_duration = duration;
                    self.legs_anim = (!self.legs_anim & RESTART_BIT) | p.anim;
                    if p.arm_countdown {
                        self.lower_body_clock = duration;
                    }
                    legs_started = true;
                } else if p.arm_countdown && new.flags & anim_flags::LOOPED != 0 {
                    self.lower_body_clock = duration;
                }
            }
            if part == Part::Legs {
                return legs_started.then_some(duration);
            }
            torso_anim = 0;
        }
        let mut force = p.force;
        let cur = clip_ordinal(self.torso_anim);
        if !force
            && cur != torso_anim
            && (!dvars::FORCE_TIMER_CHECK || self.upper_body_clock < BUSY_MS)
            && part == Part::Torso
            && table.info(cur).flags & anim_flags::RELOAD != 0
            && table.info(torso_anim).flags & anim_flags::CUTS_RELOAD != 0
        {
            force = true;
        }
        if self.upper_body_clock < BUSY_MS || force {
            if !p.cont || cur != torso_anim {
                self.torso_anim = (!self.torso_anim & RESTART_BIT) | torso_anim;
                if p.arm_countdown {
                    self.upper_body_clock = duration;
                }
                self.torso_duration = duration;
            } else if p.arm_countdown && table.info(torso_anim).flags & anim_flags::LOOPED != 0 {
                self.upper_body_clock = duration;
            }
        }
        legs_started.then_some(duration)
    }

    /// Plays every part of `cmd`, each lasting the command's length plus the busy margin.
    /// When the command's first part drives the legs, the result says whether the last part
    /// played started the legs: `Some(span)` if so.
    pub fn execute(
        &mut self,
        cmd: &Command,
        rates: [f32; 2],
        arm_timer: bool,
        keep_running: bool,
        override_busy: bool,
        table: &impl AnimTable,
    ) -> Option<i32> {
        let span = cmd.span_ms + BUSY_MS;
        let legs_lead = matches!(cmd.parts[0], Some((Part::Legs | Part::Both, _)));
        let mut reported = None;
        let present = cmd.parts.iter().zip(rates).filter_map(|(entry, rate)| entry.map(|pair| (pair, rate)));
        for ((part, anim), rate) in present {
            let request = Play {
                anim,
                span_ms: span,
                rate,
                arm_countdown: arm_timer,
                cont: keep_running,
                force: override_busy,
            };
            let started = self.play(part, &request, table);
            if legs_lead {
                reported = started;
            }
        }
        reported.map(|_| span)
    }
}

/// The gun's two times a weapon-timed anim may follow (ms).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GunTimes {
    pub gun_timer_ms: i32,
    pub weapon_ready_wait: i32,
}

/// Fire an anim event: pick the matching block, a variant by BO2's random, scale weapon-timed
/// anims, and start it. Returns the command it ran (None = nothing matched) and the legs'
/// duration when they started.
pub fn script_event<T: AnimTable>(
    body: &mut Body,
    call: AnimCall,
    conds: &T::Conds,
    table: &T,
    gun: GunTimes,
    hold_seed: &mut u32,
    movement_mode: i32,
) -> Option<(Command, Option<i32>)> {
    if call.event != AnimEvent::Death && movement_mode >= 10 {
        return None;
    }
    let mut pick = |n: usize| bg_rand(hold_seed) as usize % n;
    let cmd = table.event_command(call.event, conds, &mut pick)?;
    let mut rates = [1.0; 2];
    for (slot, entry) in cmd.parts.iter().enumerate() {
        let Some(&(_, anim)) = entry.as_ref() else { continue };
        let info = table.info(anim);
        if info.flags & anim_flags::WEAPON_TIME_SCALE != 0 {
            rates[slot] = weapon_rate(info.len_ms, gun.gun_timer_ms);
        } else if info.flags & anim_flags::WEAPON_DELAY_SCALE != 0 {
            rates[slot] = weapon_rate(info.len_ms, gun.weapon_ready_wait);
        }
    }
    let started = body.execute(&cmd, rates, true, call.cont, call.force, table);
    Some((cmd, started))
}

/// Start a movement anim: the first matching block, variant = client number mod count, continue
/// on. `Err(())` when no block matched or it has no commands; `Ok(true)` when the legs changed.
#[allow(clippy::result_unit_err)]
pub fn script_move<T: AnimTable>(
    body: &mut Body,
    mt: MoveType,
    forced: bool,
    conds: &T::Conds,
    table: &T,
    client_index: u32,
) -> Result<bool, ()> {
    let mut pick = |n: usize| client_index as usize % n;
    let cmd = table.move_command(mt, conds, &mut pick).ok_or(())?;
    Ok(body
        .execute(&cmd, [1.0; 2], false, true, forced, table)
        .is_some())
}

/// A moving player's legs each frame: his movement type, else idle.
pub fn apply_movement<T: AnimTable>(
    body: &mut Body,
    m: &MoveIn,
    conds: &T::Conds,
    table: &T,
    client_index: u32,
) -> MoveType {
    let (mt, force) = move_type(m);
    if script_move(body, mt, force, conds, table, client_index).is_err() {
        let _ = script_move(body, MoveType::Idle, false, conds, table, client_index);
        return MoveType::Idle;
    }
    mt
}

/// A still player's legs each frame: idle or a turn in place (`turn`: 0 none, 1 right, 2 left),
/// swim in deep water; idle when the block has nothing.
pub fn apply_still<T: AnimTable>(
    body: &mut Body,
    turn: usize,
    water_level: i32,
    conds: &T::Conds,
    table: &T,
    client_index: u32,
) -> MoveType {
    if water_level > 3 {
        let _ = script_move(body, MoveType::Swim, false, conds, table, client_index);
        return MoveType::Swim;
    }
    let mt = STILL_MOVES[turn.min(2)];
    if script_move(body, mt, false, conds, table, client_index).is_err() {
        let _ = script_move(body, MoveType::Idle, false, conds, table, client_index);
        return MoveType::Idle;
    }
    mt
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny script: anim 1 idle legs, 2 run legs (moves), 3 fire torso, 4 reload torso,
    /// 5 drop torso (cuts reload, weapon-timed), 6 crouch stance legs, 7 dive both.
    struct Table;

    const IDLE: [Command; 1] = [Command {
        parts: [Some((Part::Legs, 1)), None],
        span_ms: 1000,
    }];
    const RUN: [Command; 2] = [
        Command {
            parts: [Some((Part::Legs, 2)), None],
            span_ms: 700,
        },
        Command {
            parts: [Some((Part::Legs, 8)), None],
            span_ms: 700,
        },
    ];
    const TRIGGER_SLOT: [Command; 1] = [Command {
        parts: [Some((Part::Torso, 3)), None],
        span_ms: 150,
    }];
    const RELOAD: [Command; 1] = [Command {
        parts: [Some((Part::Torso, 4)), None],
        span_ms: 2500,
    }];
    const DROP: [Command; 1] = [Command {
        parts: [Some((Part::Torso, 5)), None],
        span_ms: 400,
    }];
    const DUCK_BUTTON_BIT: [Command; 1] = [Command {
        parts: [Some((Part::Legs, 6)), None],
        span_ms: 600,
    }];
    const DIVE: [Command; 1] = [Command {
        parts: [Some((Part::Both, 7)), None],
        span_ms: 800,
    }];

    fn choose(cmds: &[Command], pick: &mut dyn FnMut(usize) -> usize) -> Option<Command> {
        (!cmds.is_empty()).then(|| cmds[pick(cmds.len())])
    }

    impl AnimTable for Table {
        type Conds = Conds;
        fn event_command(
            &self,
            event: AnimEvent,
            _: &Conds,
            pick: &mut dyn FnMut(usize) -> usize,
        ) -> Option<Command> {
            let cmds: &[Command] = match event {
                AnimEvent::FireWeapon => &TRIGGER_SLOT,
                AnimEvent::Reload => &RELOAD,
                AnimEvent::DropWeapon => &DROP,
                AnimEvent::StandToCrouch => &DUCK_BUTTON_BIT,
                AnimEvent::DtpTakeoff => &DIVE,
                _ => return None,
            };
            choose(cmds, pick)
        }
        fn move_command(
            &self,
            mt: MoveType,
            _: &Conds,
            pick: &mut dyn FnMut(usize) -> usize,
        ) -> Option<Command> {
            let cmds: &[Command] = match mt {
                MoveType::Idle => &IDLE,
                MoveType::Run => &RUN,
                _ => return None,
            };
            choose(cmds, pick)
        }
        fn info(&self, anim: u16) -> AnimInfo {
            let (len_ms, flags, moves) = match anim {
                2 | 8 => (700, anim_flags::RUN, true),
                3 => (150, anim_flags::TRIGGER_SLOT, false),
                4 => (2500, anim_flags::RELOAD, false),
                5 => (
                    400,
                    anim_flags::CUTS_RELOAD | anim_flags::WEAPON_TIME_SCALE,
                    false,
                ),
                6 => (600, anim_flags::STANCE, false),
                _ => (1000, 0, false),
            };
            AnimInfo {
                len_ms,
                flags,
                moves,
            }
        }
    }

    fn fire(body: &mut Body, call: AnimCall, seed: &mut u32) -> Option<(Command, Option<i32>)> {
        let gun = GunTimes {
            gun_timer_ms: 400,
            weapon_ready_wait: 0,
        };
        script_event(body, call, &Conds::default(), &Table, gun, seed, 0)
    }

    #[test]
    fn bo2_rand_is_the_lcg_top_15_bits() {
        let mut seed = 1;
        assert_eq!(bg_rand(&mut seed), 20);
        assert_eq!(bg_rand(&mut seed), 25617);
    }

    #[test]
    fn event_sets_timer_duration_plus_50_and_flips_restart() {
        let mut body = Body::default();
        let mut seed = 0;
        fire(&mut body, AnimCall::TRIGGER_SLOT, &mut seed);
        assert_eq!(clip_ordinal(body.torso_anim), 3);
        assert_eq!(body.upper_body_clock, 200);
        let bit = body.torso_anim & RESTART_BIT;
        body.tick(200);
        fire(&mut body, AnimCall::TRIGGER_SLOT, &mut seed);
        assert_ne!(
            body.torso_anim & RESTART_BIT,
            bit,
            "a new shot restarts the anim"
        );
    }

    #[test]
    fn busy_part_keeps_its_anim_unless_forced() {
        let mut body = Body::default();
        let mut seed = 0;
        fire(&mut body, AnimCall::RELOAD, &mut seed);
        // Unforced, not a cut: the reload holds.
        let call = AnimCall::new(AnimEvent::FireWeapon, false, false);
        fire(&mut body, call, &mut seed);
        assert_eq!(clip_ordinal(body.torso_anim), 4);
        // A forced shot takes it.
        fire(&mut body, AnimCall::TRIGGER_SLOT, &mut seed);
        assert_eq!(clip_ordinal(body.torso_anim), 3);
    }

    #[test]
    fn swap_cuts_a_reload() {
        let mut body = Body::default();
        let mut seed = 0;
        fire(&mut body, AnimCall::RELOAD, &mut seed);
        assert_eq!(body.upper_body_clock, 2550);
        fire(&mut body, AnimCall::DROP, &mut seed);
        assert_eq!(clip_ordinal(body.torso_anim), 5);
    }

    #[test]
    fn drop_plays_over_the_weapon_time_clamped() {
        // 400 ms anim over a 400 ms drop: rate 1, 450 ms.
        let mut body = Body::default();
        let mut seed = 0;
        fire(&mut body, AnimCall::DROP, &mut seed);
        assert_eq!(body.upper_body_clock, 450);
        assert!((weapon_rate(400, 100) - 1.66).abs() < 1e-6);
        assert!((weapon_rate(400, 10_000) - 0.34).abs() < 1e-6);
        assert!((weapon_rate(400, 0) - 1.66).abs() < 1e-6);
    }

    #[test]
    fn movement_takes_only_free_legs_and_continues() {
        let mut body = Body::default();
        let m = MoveIn {
            xy_speed: 190.0,
            ..MoveIn::default()
        };
        assert_eq!(
            apply_movement(&mut body, &m, &Conds::default(), &Table, 0),
            MoveType::Run
        );
        assert_eq!(clip_ordinal(body.legs_anim), 2);
        assert_eq!(body.lower_body_clock, 0, "movement never sets the timer");
        let bit = body.legs_anim & RESTART_BIT;
        apply_movement(&mut body, &m, &Conds::default(), &Table, 0);
        assert_eq!(body.legs_anim & RESTART_BIT, bit, "same anim continues");
    }

    #[test]
    fn movement_variant_is_client_number() {
        let mut body = Body::default();
        let m = MoveIn {
            xy_speed: 190.0,
            ..MoveIn::default()
        };
        apply_movement(&mut body, &m, &Conds::default(), &Table, 3);
        assert_eq!(clip_ordinal(body.legs_anim), 8);
    }

    #[test]
    fn stance_anim_gives_way_to_running_after_200ms() {
        let mut body = Body::default();
        let mut seed = 0;
        let call = AnimCall::new(AnimEvent::StandToCrouch, false, true);
        fire(&mut body, call, &mut seed);
        assert_eq!(body.lower_body_clock, 650);
        let m = MoveIn {
            xy_speed: 190.0,
            ..MoveIn::default()
        };
        body.tick(100);
        apply_movement(&mut body, &m, &Conds::default(), &Table, 0);
        assert_eq!(clip_ordinal(body.legs_anim), 6, "too early");
        body.tick(150);
        apply_movement(&mut body, &m, &Conds::default(), &Table, 0);
        assert_eq!(clip_ordinal(body.legs_anim), 2);
    }

    #[test]
    fn both_drives_the_body_and_clears_the_torso() {
        let mut body = Body::default();
        let mut seed = 0;
        fire(&mut body, AnimCall::TRIGGER_SLOT, &mut seed);
        body.tick(500);
        fire(&mut body, AnimCall::DTP_TAKEOFF, &mut seed);
        assert_eq!(clip_ordinal(body.legs_anim), 7);
        assert_eq!(clip_ordinal(body.torso_anim), 0);
    }

    #[test]
    fn unmatched_move_falls_back_to_idle() {
        let mut body = Body::default();
        let m = MoveIn {
            xy_speed: 50.0,
            stance: 2,
            ..MoveIn::default()
        };
        assert_eq!(
            apply_movement(&mut body, &m, &Conds::default(), &Table, 0),
            MoveType::Idle
        );
        assert_eq!(clip_ordinal(body.legs_anim), 1);
    }

    #[test]
    fn move_types_follow_bo2s_table() {
        let stand = |xy_speed, hurt| MoveIn {
            xy_speed,
            hurt,
            ..MoveIn::default()
        };
        assert_eq!(move_type(&stand(200.0, false)).0, MoveType::Run);
        assert_eq!(move_type(&stand(100.0, false)).0, MoveType::Walk);
        assert_eq!(move_type(&stand(100.0, true)).0, MoveType::StumbleWalk);
        assert_eq!(
            move_type(&stand(20.0, false)).0,
            MoveType::Walk,
            "shuffle off: walk"
        );
        assert_eq!(move_type(&stand(200.0, true)).0, MoveType::Stumble);
        let sprint = MoveIn {
            sprinting: true,
            ..stand(190.0, false)
        };
        assert_eq!(move_type(&sprint), (MoveType::Sprint, true));
        let slow = MoveIn {
            sprinting: true,
            ..stand(150.0, false)
        };
        assert_eq!(move_type(&slow), (MoveType::Run, false));
        let back = MoveIn {
            sprinting: true,
            backward: true,
            ..stand(190.0, false)
        };
        assert_eq!(move_type(&back).0, MoveType::Run, "no sprint backward");
        let crouch = MoveIn {
            stance: 1,
            ..stand(80.0, false)
        };
        assert_eq!(move_type(&crouch).0, MoveType::Run);
        let crouch_walk = MoveIn {
            walking: true,
            ..crouch
        };
        assert_eq!(move_type(&crouch_walk).0, MoveType::Walk);
        let prone = MoveIn {
            stance: 2,
            ..stand(30.0, true)
        };
        assert_eq!(move_type(&prone).0, MoveType::Walk);
        let dive = MoveIn {
            diving: true,
            ..sprint
        };
        assert_eq!(move_type(&dive).0, MoveType::Dtp);
        let slide = MoveIn {
            sliding: true,
            ..sprint
        };
        assert_eq!(move_type(&slide).0, MoveType::Slide);
        let swim = MoveIn {
            water_level: 4,
            ..slide
        };
        assert_eq!(move_type(&swim).0, MoveType::Swim);
    }

    #[test]
    fn melee_choice_in_bo2_order() {
        assert_eq!(melee_event(true, true, false, true), AnimEvent::MeleeAttack);
        assert_eq!(
            melee_event(false, true, true, true),
            AnimEvent::KnifeMeleeCharge
        );
        assert_eq!(melee_event(false, false, true, true), AnimEvent::MeleeLeft);
        assert_eq!(
            melee_event(false, false, false, true),
            AnimEvent::KnifeMelee
        );
        assert_eq!(
            melee_event(false, true, false, false),
            AnimEvent::MeleeAttack
        );
        assert!(is_lunge(true, 120.0, 64.0));
        assert!(!is_lunge(true, 60.0, 64.0));
    }

    #[test]
    fn dual_reload_needs_both_hands_early() {
        let r = state::RELOADING;
        assert!(dual_reload_shows(r, r, 1500, 1500, 2000, 2000));
        assert!(!dual_reload_shows(r, r, 900, 1500, 2000, 2000));
        assert!(!dual_reload_shows(r, state::READY, 1500, 0, 2000, 2000));
    }

    #[test]
    fn conditions() {
        assert_eq!(move_status(150.0, false), 2);
        assert_eq!(move_status(50.0, false), 1);
        assert_eq!(move_status(1.0, false), 0);
        assert_eq!(move_status(150.0, true), 0);
        assert_eq!(weapon_position(true, state::RIOTSHIELD_HOLD), 1);
        assert_eq!(weapon_position(false, state::RIOTSHIELD_HOLD), 2);
        assert_eq!(weapon_position(false, state::READY), 0);
        assert_eq!(slope(31.0), 1);
        assert_eq!(slope(-31.0), 2);
        assert!(first_raise(false, 0));
        assert!(!first_raise(false, 7));
        assert!(!first_raise(true, 0));
        assert!(run_to_idle(false, false, 120.0));
        assert!(!run_to_idle(true, false, 120.0));
        assert_eq!(
            AnimCall::from_script(crate::events::AnimScript::FirstRaiseWeapon).event,
            AnimEvent::FirstRaiseWeapon
        );
    }

    #[test]
    fn gun_code_calls_carry_bo2s_flags() {
        use crate::events::AnimScript;
        let call = |s| {
            let c = AnimCall::from_script(s);
            (c.event, c.cont, c.force)
        };
        assert_eq!(call(AnimScript::FireWeapon), (AnimEvent::FireWeapon, false, true));
        assert_eq!(
            call(AnimScript::RiotshieldStart),
            (AnimEvent::FireWeapon, false, true)
        );
        assert_eq!(call(AnimScript::OffHandStart), (AnimEvent::FireWeapon, false, true));
        assert_eq!(call(AnimScript::PrimeGrenade), (AnimEvent::PrimeGrenade, true, true));
        assert_eq!(call(AnimScript::PrimeGear), (AnimEvent::RaiseWeapon, true, true));
        assert_eq!(call(AnimScript::Reload), (AnimEvent::Reload, false, true));
        assert_eq!(call(AnimScript::DropWeapon), (AnimEvent::DropWeapon, false, false));
        assert_eq!(call(AnimScript::RaiseWeapon), (AnimEvent::RaiseWeapon, false, false));
        assert_eq!(call(AnimScript::DtpTakeoff), (AnimEvent::DtpTakeoff, false, true));
        assert_eq!(call(AnimScript::DtpLand), (AnimEvent::DtpLand, true, true));
        assert_eq!(call(AnimScript::RunToIdle), (AnimEvent::RunToIdle, false, false));
        assert_eq!(
            call(AnimScript::KnifeMeleeCharge),
            (AnimEvent::KnifeMeleeCharge, false, true)
        );
    }

    #[test]
    fn dead_players_fire_only_death() {
        let mut body = Body::default();
        let mut seed = 0;
        let gun = GunTimes::default();
        assert!(
            script_event(
                &mut body,
                AnimCall::TRIGGER_SLOT,
                &Conds::default(),
                &Table,
                gun,
                &mut seed,
                10
            )
            .is_none()
        );
    }
}
