//! bo2mp: players' bodies as Black Ops II shows them to others.
//!
//! - The body the scripts give a player (`setmodel` / `attach` in BO2's
//!   `mpbody/class_*` faction scripts) goes to every client as a client
//!   value (`bo2mp_body`) of that player.
//! - His third-person animations are picked as the game picks them, from
//!   its own `mp/playeranim.script`: each tick the movement type's first
//!   matching item (idle, walk, run, sprint, climbing...) plays on the legs
//!   and torso that no event holds; events (firing, reloading, a weapon
//!   switch, melee, jumping, landing, stance changes, death) play their
//!   item for its duration or the animation's length. The choice goes out
//!   in his state's `legs_anim` / `torso_anim` (an animation's place in the
//!   script's list, see [`crate::body_anim_script`]).
//! - `cloneplayer` leaves the engine's corpse (it keeps his death
//!   animation) and gives the scripts an entity for it.

use super::pairs::KeepByValue;
use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::prelude::{Resource, World};
use gsc_t6::{ObjKind, ScriptValue, Vm};
use movement_t6::bridge::{dive_in_air, dive_sliding};
use playerstate_t6::game::{AvatarState, ledge_climb_bits, movement_state_bits};
use weapon_t6::state;

use super::{Ent, Zm, frame};
use crate::body_anim_script::{
    T6AnimCommand, T6AnimFacts, T6AnimPart, T6PlayerAnims, t6_anim_index, t6_move_direction,
    t6_anim_part, t6_pack_anim, t6_weapon_class_name, t6_with_duration, t6_with_legs_yaw,
    t6_slope, t6_with_offhand, T6_ANIM_TOGGLE,
};
use crate::sim_parts::ClientId;
use weapon_t6::third;

#[path = "third_table.rs"]
mod third_table;

/// The client value that carries a player's body: `<model>` then
/// `;<model>@<tag>` per attached model.
pub(crate) const BODY_DVAR: &str = "bo2mp_body";

/// Speed (units/s) under which a player stands still (the engine's
/// `PLAYER_MOVE_THRESHHOLD`).
const MOVE_THRESHOLD: f32 = 10.0;

#[derive(Clone, Debug, Default)]
struct Track {
    present: bool,
    /// How long an event still holds the legs and the torso (ms): the
    /// engine's legs and torso timers, kept here (a frozen player's movement
    /// does not run them down).
    holds: [i32; 2],
    /// Where his legs face (world yaw, degrees): they lag his view when he
    /// turns standing still (BO2's turn-in-place) and twist toward a
    /// diagonal move; the torso keeps to the view.
    legs_yaw: f32,
    /// Turning in place: 1 right, -1 left, 0 not.
    turning: i8,
    /// The legs are swinging to their goal (BG_SwingAngles' `swinging`).
    legs_swinging: bool,
    /// His view's yaw last tick (degrees): which way a turn in place goes.
    last_view: f32,
    /// The dive to prone last tick: 0 none, 1 in the air, 2 sliding.
    dive: u8,
    /// Mantling last tick.
    mantling: bool,
    /// The last way his stick pushed (BO2's `prevdirection`): empty before any.
    prev_dir: &'static str,
    stance: u8,
    was_grounded: bool,
    arm: u32,
    wstate: i32,
    clip: i32,
    /// When the legs' and the torso's animations started (their value
    /// without the extra bits, level ms), for posing his hit boxes.
    anim_start: [(i32, i64); 2],
    /// BO2's legs / torso slots and timers (window 14's third-person rules).
    body: third::Body,
    /// BO2's hold-rand seed: picks an event's variant.
    seed: u32,
    /// The ground pitch his last command's moves averaged (BO2's
    /// averagePitch), kept until his next command: the legs' `slope`.
    slope_pitch: f32,
}

#[derive(Resource, Default)]
pub(crate) struct Bodies {
    pub script: Option<Arc<T6PlayerAnims>>,
    tracks: BTreeMap<u32, Track>,
    /// The script entity each player's last body (`cloneplayer`) is.
    corpses: BTreeMap<u32, u32>,
    /// Each of BO2's four body slots' new-body bit, flipped every time the slot takes a new
    /// body, so the clients never take it for the body it replaced.
    corpse_laps: [bool; BO2_PLAYER_CORPSES],
    /// Players whose death animation is set (this life's).
    died: std::collections::BTreeSet<u32>,
    /// Each player's look as last sent.
    looks: BTreeMap<u32, BodyLook>,
    /// Posing hit boxes: (total us, most us, times) since the last report.
    hit_us: (u64, u64, u64),
    /// Each body entity's death animation (`getcorpseanim`).
    corpse_anims: BTreeMap<u32, String>,
    /// Each body entity's engine corpse: (its slot, that corpse's entity
    /// number), for `startragdoll` and `launchragdoll`.
    corpse_slots: BTreeMap<u32, (u8, i32)>,
    rng: u32,
    logged: bool,
    /// What BO2 marks on each script anim (window 14), worked out once: `Some(None)` when
    /// the script does not fit BO2's anim value.
    third_info: Option<Option<Arc<Vec<third::AnimInfo>>>>,
    /// The legs' jump and land BO2's movement started since each player's
    /// last body frame.
    moved: BTreeMap<u32, movement_t6::BodyAnims>,
    /// BO2's movement runs: its jump and land replace the guess from the
    /// ground.
    t6_move: bool,
}

impl Bodies {
    pub(crate) fn new(script: Option<Arc<T6PlayerAnims>>) -> Self {
        Self {
            script,
            ..Self::default()
        }
    }

    fn next_rand(&mut self) -> u32 {
        // xorshift32: the variant a multi-animation item plays.
        let seed = self.rng.max(1);
        let a = seed ^ (seed << 13);
        let b = a ^ (a >> 17);
        let next = b ^ (b << 5);
        self.rng = next;
        next
    }
}

/// How a player looks to the others: the body model and attachments his
/// scripts chose, and the gun stowed on his back (`setstowedweapon`).
#[derive(Clone, Debug, Default)]
pub(crate) struct BodyLook {
    pub mesh: String,
    pub attached: Vec<(String, String)>,
    /// (weapon index, weapon name).
    pub stowed: Option<(u32, String)>,
    /// The riot shield he carries: in his left hand while it is his
    /// weapon, on his back while he has it (`refreshshieldattachment`).
    pub shield: Option<Held>,
    /// The piece of equipment in his hand while the scripts show it
    /// (`setoffhandvisible`, window 5).
    pub inhand: Option<Held>,
}

/// A model he carries on one of his tags.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    pub mesh: String,
    pub bone: &'static str,
}

/// The tag BO2 draws the stowed gun on.
const STOWED_TAG: &str = "tag_stowed_back";

/// Change a player's look and send it: `<model>`, then `;<model>@<tag>`
/// per attachment, then `;#<weapon index>@tag_stowed_back` for the stowed
/// gun (the client draws that weapon's world model and attachments).
pub(crate) fn edit_body(game: &mut World, peer: u32, fl: impl FnOnce(&mut BodyLook)) {
    let mut look = game
        .get_resource::<Bodies>()
        .and_then(|b| b.looks.get(&peer).cloned())
        .unwrap_or_default();
    fl(&mut look);
    let mut v = look.mesh.clone();
    for (m, tag) in &look.attached {
        v.push(';');
        v.push_str(m);
        v.push('@');
        v.push_str(tag);
    }
    if let Some((wd, _)) = &look.stowed {
        v.push_str(&format!(";#{wd}@{STOWED_TAG}"));
    }
    for Held { mesh, bone } in look.shield.iter().chain(&look.inhand) {
        v.push_str(&format!(";{mesh}@{bone}"));
    }
    if let Some(mut b) = game.get_resource_mut::<Bodies>() {
        b.looks.insert(peer, look);
    }
    super::set_client_dvar(game, peer, BODY_DVAR, &v);
}

/// The riot shield on him: drawn on his body (model, tag) and, for the sim,
/// stopping what comes at the side it faces (in his hands: his front; on
/// his back: his back).
fn set_shield(game: &mut World, cid: ClientId, shield: Option<Held>) {
    // (Multiplayer only: BO2's zombies shield gives no blast cover, and no
    // one there shoots at him.)
    let mp = game.get_resource::<Zm>().is_some_and(|z| z.mp);
    let held = shield.as_ref().map(|s| s.bone != STOWED_TAG).filter(|_| mp);
    frame(game).set_riot_shield(cid, held);
    edit_body(game, cid.0, |l| l.shield = shield);
}

/// Everything attached to him comes off (`detachall`), the riot shield too.
pub(crate) fn detach_all(world: &mut World, client: u32) {
    frame(world).set_riot_shield(ClientId(client), None);
    edit_body(world, client, |l| {
        l.attached.clear();
        l.shield = None;
    });
}

/// The weapon stowed on a player's back (`getstowedweapon`).
pub(crate) fn stowed_of(world: &World, client: u32) -> Option<String> {
    world
        .get_resource::<Bodies>()
        .and_then(|b| b.looks.get(&client))
        .and_then(|l| l.stowed.as_ref())
        .map(|(_, name)| name.clone())
}

fn stance_of(ps: &AvatarState) -> u8 {
    if ps.movement_state_bits & movement_state_bits::LYING_FLAT != 0 {
        2
    } else if ps.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT != 0 {
        1
    } else {
        0
    }
}

const STANCE_NAMES: [&str; 3] = ["stand", "crouch", "prone"];

/// The gun's timers for the third-person rules: `weapon_time` as given, the
/// delay as the player state holds it.
fn gun_times(ps: &AvatarState, weapon_time: i32) -> third::GunTimes {
    third::GunTimes { gun_timer_ms: weapon_time, weapon_ready_wait: ps.weapon_ready_wait }
}

fn xy_speed(ps: &AvatarState) -> f32 {
    (ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1]).sqrt()
}

fn has_ground(ps: &AvatarState) -> bool {
    ps.standing_on != playerstate_t6::game::NO_ENTITY_SLOT
}

fn movestatus(ps: &AvatarState) -> &'static str {
    if xy_speed(ps) <= MOVE_THRESHOLD {
        "stationary"
    } else if ps.movement_state_bits & (movement_state_bits::WALKING | movement_state_bits::LYING_FLAT) != 0 {
        "walk"
    } else {
        "run"
    }
}

/// The movement type the script is read for, as the engine picks it
/// (`None` in the air: the jump's animation holds).
fn movetype(ps: &AvatarState) -> Option<&'static str> {
    if dive_phase(ps) != 0 || ps.movement_state_bits & movement_state_bits::MANTLE != 0 {
        return None;
    }
    if ps.movement_state_bits & movement_state_bits::LADDER != 0 {
        // Down while he goes down, else up (T5 PM_LadderMove anim pick:
        // climbdown when velocity z < 0; a still climber's climb stands
        // still, its rate follows his climbing speed).
        return Some(if ps.velocity[2] < 0.0 { "climbdown" } else { "climbup" });
    }
    if !has_ground(ps) {
        return None;
    }
    if xy_speed(ps) <= MOVE_THRESHOLD {
        return Some("idle");
    }
    if ps.movement_state_bits & movement_state_bits::SPRINTING != 0 && stance_of(ps) == 0 {
        return Some("sprint");
    }
    if ps.movement_state_bits & (movement_state_bits::WALKING | movement_state_bits::LYING_FLAT) != 0 {
        return Some("walk");
    }
    Some("run")
}

/// `player_runThreshhold` (BO2 default): the speed a stop from a run needs.
const RUN_THRESHOLD: f32 = 110.0;
/// `player_strafeAnimCosAngle` (BO2 default).
const STRAFE_ANIM_COS: f32 = 0.5;

/// The way the stick pushes, in the script's words (BO2's `direction` from the move command):
/// empty when let go.
fn stick_direction(stick: [i8; 2]) -> &'static str {
    let (fwd, right) = (f32::from(stick[0]), f32::from(stick[1]));
    let mag = fwd.hypot(right);
    if !(mag > 0.0) {
        return "";
    }
    let (fwd, right) = (fwd / mag, right / mag);
    if fwd > STRAFE_ANIM_COS {
        "forward"
    } else if fwd < -STRAFE_ANIM_COS {
        "backward"
    } else if right > 0.0 {
        "right"
    } else if right < 0.0 {
        "left"
    } else {
        ""
    }
}

/// The dive to prone (BO2's dolphin dive): 0 none, 1 in the air, 2 sliding
/// into prone on the ground.
fn dive_phase(ps: &AvatarState) -> u8 {
    if dive_in_air(ps) {
        1
    } else if dive_sliding(ps) {
        2
    } else {
        0
    }
}

/// The script's mantle movement types by the engine's mantle transition (the
/// same clips the mantle moves the player by): up, by ledge height, then over.
const MANTLE_UP_MOVES: [&str; 7] = [
    "mantle_up_57",
    "mantle_up_51",
    "mantle_up_45",
    "mantle_up_39",
    "mantle_up_33",
    "mantle_up_27",
    "mantle_up_21",
];
const MANTLE_OVER_MOVES: [&str; 3] = ["mantle_over_high", "mantle_over_mid", "mantle_over_low"];

/// The animation a mantle plays now and how long its phase still has, as
/// BO2's Mantle_Move picks it each tick: the script's `mantle_up_*` move while
/// the movement still climbs (its own up length, the fast mantle's shorter
/// climb included), then its `mantle_over_*` move for a mantle over. `facts`
/// carry the fast-mantle perk (its `_fast` clips, some on the legs alone); the
/// variant is the client number's.
fn mantle_command(
    sim: &mut World,
    script: &T6PlayerAnims,
    ps: &AvatarState,
    facts: &T6AnimFacts<'_>,
    client: u32,
) -> Option<(T6AnimCommand, i32)> {
    let shift_offset = ps.mantle_state.trans_index;
    let lengths = frame(sim).animation_refs();
    let up_len = movement_t6::bridge::mantle::up_length(ps, &*lengths);
    let elapsed = ps.mantle_state.timer;
    let (movetype, left_ms) = if elapsed <= up_len || ps.mantle_state.flags & ledge_climb_bits::OVER == 0 {
        let i = usize::try_from(movement_t6::bridge::mantle::trans_up_anim(shift_offset) - 1).ok()?;
        (*MANTLE_UP_MOVES.get(i)?, up_len - elapsed)
    } else {
        let i = usize::try_from(movement_t6::bridge::mantle::trans_over_anim(shift_offset) - 8).ok()?;
        let over_len = movement_t6::bridge::mantle::over_length(ps, &*lengths);
        (*MANTLE_OVER_MOVES.get(i)?, up_len + over_len - elapsed)
    };
    let commands = script.pick_move(movetype, facts)?;
    let command = *commands.get(client as usize % commands.len().max(1))?;
    Some((command, left_ms.max(50)))
}

/// `fastmantle` for a mantle the fast-mantle perk speeds (the script's only
/// `perk` condition, read only in its mantle moves), else empty.
fn mantle_perk(ps: &AvatarState) -> &'static str {
    if ps.mantle_state.flags & ledge_climb_bits::FAST_MANTLE != 0 {
        "fastmantle"
    } else {
        ""
    }
}

/// How long the weapon takes for the action an event starts (ms): the
/// time left on the weapon (`weapon_time`, which the reload, raise or drop
/// just set, perks included); a segmented reload (a shotgun's shell by
/// shell) adds its shells and its end.
fn weapon_action_ms(game: &mut World, pstate: &AvatarState, ev_name: &str, clip_no: i32) -> Option<i32> {
    let facts = frame(game).damage_profile_of(pstate.weapon)?;
    let left = pstate.weapon_timer.max(0);
    let ms = if ev_name == "reload" && facts.staged_reload {
        let shells = (facts.magazine_capacity - clip_no).max(1);
        facts.refill_begin_millis.max(left) + shells * facts.reload_duration_millis + facts.refill_finish_millis
    } else if left > 0 {
        left
    } else {
        match ev_name {
            "reload" if clip_no == 0 => facts.refill_dry_millis,
            "reload" => facts.reload_duration_millis,
            "dropweapon" => facts.holster_ms,
            _ => facts.draw_ms,
        }
    };
    (ms > 0).then_some(ms + crate::SIM_STEP_MILLIS as i32)
}

/// The weapon facts the script reads: its anim type and class.
struct GunWords {
    anim: String,
    class: &'static str,
}

/// The weapon facts the script reads, for `gun`.
fn weapon_words(game: &mut World, script: &T6PlayerAnims, gun: u32) -> GunWords {
    let facts = frame(game).damage_profile_of(gun);
    let anim_type = facts.map_or(1, |f| f.player_motion_kind);
    let class = i32::from(frame(game).gun_facts().get(gun).core.weap_class);
    GunWords {
        anim: script.anim_type_name(anim_type).to_owned(),
        class: t6_weapon_class_name(class),
    }
}

fn anim_length_ms(world: &World, script: &T6PlayerAnims, anim: u16) -> i32 {
    let Some(name) = script.anims.get(usize::from(anim)) else {
        return 0;
    };
    world
        .resource::<Zm>()
        .anims
        .get(name)
        .filter(|a| a.fps > 0.0)
        .map_or(500, |a| ((f32::from(a.frame_total) / a.fps) * 1000.0) as i32)
}

/// Start a command on the parts it names. An event restarts its animation
/// (the toggle bit flips) and holds the parts for its time; a movement
/// animation only takes parts no event holds and keeps one already playing.
fn apply(ps: &mut AvatarState, holds: &mut [i32; 2], command: &T6AnimCommand, hold_ms: Option<i32>) {
    let legs = matches!(command.part, T6AnimPart::Legs | T6AnimPart::Both);
    let torso = matches!(command.part, T6AnimPart::Torso | T6AnimPart::Both);
    let set = |value: &mut i32, timer: &mut i32| match hold_ms {
        Some(ms) => {
            let toggle = *value & T6_ANIM_TOGGLE == 0;
            *value = t6_pack_anim(command.anim, toggle);
            *timer = ms.max(0);
        }
        None => {
            if *timer > 0 || t6_anim_index(*value) == Some(command.anim) {
                return;
            }
            let toggle = *value & T6_ANIM_TOGGLE != 0;
            *value = t6_pack_anim(command.anim, !toggle);
        }
    };
    let [legs_hold, torso_hold] = holds;
    if legs {
        set(&mut ps.legs_anim, legs_hold);
    }
    if torso {
        set(&mut ps.torso_anim, torso_hold);
    }
}

/// Legs that start turning in place once the view is this far from them.
const TURN_START_DEG: f32 = 45.0;
/// Legs never trail the view by more than this.
const TURN_MAX_DEG: f32 = 90.0;
/// How fast the legs come round turning in place (degrees/s).
const TURN_SPEED_DEG: f32 = 180.0;
/// BO2's `bg_swingSpeed`.
const SWING_SPEED: f32 = 0.15;
/// BO2's `bg_legYawTolerance` and `bg_legYawCrouchTolerance`: how far the legs may sit off their goal before they swing.
const LEG_YAW_TOLERANCE: f32 = 40.0;
const LEG_YAW_CROUCH_TOLERANCE: f32 = 10.0;
/// The legs never sit further than this from their goal (BG_PlayerAngles).
const LEG_YAW_CLAMP: f32 = 150.0;
/// How fast the legs twist to a crawl's direction (degrees/s).
const PRONE_TWIST_SPEED_DEG: f32 = 540.0;

fn norm180(mut a: f32) -> f32 {
    while a > 180.0 {
        a -= 360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a
}

fn approach(from: f32, to: f32, step: f32) -> f32 {
    if (to - from).abs() <= step { to } else { from + step * (to - from).signum() }
}

/// BG_SwingAngles: `angle` stays put while it is within
/// `tolerance` of `dest`; once off, it swings there (`swinging` holds) by
/// `max(0.5, |gap| * 0.05) * frame_ms * speed` a frame, and is never more than
/// `clamp` from it.
fn swing_angles(dest: f32, tolerance: f32, clamp: f32, speed: f32, frame_ms: f32, angle: &mut f32, swinging: &mut bool) {
    if !*swinging {
        if norm180(*angle - dest).abs() <= tolerance {
            return;
        }
        *swinging = true;
    }
    let gap = norm180(dest - *angle);
    let step = (gap.abs() * 0.05).max(0.5) * frame_ms * speed;
    let moved = if step < gap.abs() {
        step * gap.signum()
    } else {
        *swinging = false;
        gap
    };
    *angle = (*angle + moved).rem_euclid(360.0);
    let gap = norm180(dest - *angle);
    if gap > clamp {
        *angle = (dest - clamp).rem_euclid(360.0);
    } else if gap < -clamp {
        *angle = (dest + clamp).rem_euclid(360.0);
    }
}

/// One tick of the legs' yaw: moving, they swing to BO2's goal (a strafe
/// animation's legs face the view, any other the view plus his movement
/// direction); standing, they stay put until the view is `TURN_START_DEG`
/// away, then turn in place to it. `strafe`: his legs play a `direction
/// left/right` animation. Returns the legs' offset from the view (degrees).
fn legs_yaw_step(track: &mut Track, ps: &AvatarState, ground: bool, strafe: bool) -> f32 {
    let view = ps.look_angles[1];
    let dt = crate::SIM_STEP_MILLIS as f32 / 1000.0;
    if !track.present {
        track.legs_yaw = view;
        track.last_view = view;
        track.turning = 0;
    }
    let view_moved = norm180(track.last_view - view);
    track.last_view = view;
    let mut off = norm180(track.legs_yaw - view);
    if ps.movement_state_bits & movement_state_bits::LADDER != 0 {
        // On a ladder he faces it (its normal points out at him).
        track.turning = 0;
        let v = ps.ladder_normal_dir;
        if v[0] * v[0] + v[1] * v[1] > 1e-4 {
            off = norm180((-v[1]).atan2(-v[0]).to_degrees() - view);
        }
        track.legs_yaw = view + off;
        return off;
    }
    let flags = ps.e_flags;
    let prone = flags & playerstate_t6::game::entity_flag_bits::LYING_FLAT != 0;
    if ground && !prone && xy_speed(ps) > MOVE_THRESHOLD {
        track.turning = 0;
        // BG_PlayerAngles: BG_AnimParseAnimScript flags a
        // `direction left/right` animation 0x10/0x20, and its legs
        // swing to the view with no tolerance; any other goes to the view plus
        // the movement direction (PM_SetMovementDir), with the leg-yaw
        // tolerance until it starts to swing.
        let (dest, tolerance) = if strafe {
            track.legs_swinging = false;
            (view, 0.0)
        } else if track.legs_swinging {
            (view + ps.move_heading as f32, 0.0)
        } else if flags & playerstate_t6::game::entity_flag_bits::CROUCH_BIT != 0 {
            (view + ps.move_heading as f32, LEG_YAW_CROUCH_TOLERANCE)
        } else {
            (view + ps.move_heading as f32, LEG_YAW_TOLERANCE)
        };
        let frame_ms = crate::SIM_STEP_MILLIS as f32;
        swing_angles(dest, tolerance, LEG_YAW_CLAMP, SWING_SPEED, frame_ms, &mut track.legs_yaw, &mut track.legs_swinging);
        return norm180(track.legs_yaw - view);
    } else if ground && xy_speed(ps) > MOVE_THRESHOLD {
        // Crawling prone: the legs twist toward the crawl.
        track.turning = 0;
        let heading = ps.velocity[1].atan2(ps.velocity[0]).to_degrees();
        let want = norm180(heading - view).clamp(-45.0, 45.0);
        off = approach(off, want, PRONE_TWIST_SPEED_DEG * dt);
    } else if ground {
        if track.turning != 0 {
            // BO2's PM_Footsteps_TurnAnim: the turn anim follows the way the view
            // moved this frame (right when the yaw fell), and keeps on when it did not.
            if view_moved > 0.0 {
                track.turning = 1;
            } else if view_moved < 0.0 {
                track.turning = -1;
            }
            off = approach(off, 0.0, TURN_SPEED_DEG * dt);
            if off.abs() < 1.0 {
                off = 0.0;
                track.turning = 0;
            }
        } else if off.abs() > TURN_START_DEG {
            // Legs left of the view: he turned right.
            track.turning = if off > 0.0 { 1 } else { -1 };
        }
    }
    off = off.clamp(-TURN_MAX_DEG, TURN_MAX_DEG);
    track.legs_yaw = view + off;
    off
}

fn pick(bodies: &mut Bodies, commands: &[T6AnimCommand]) -> Option<T6AnimCommand> {
    if commands.is_empty() {
        return None;
    }
    let r = bodies.next_rand() as usize;
    Some(commands[r % commands.len()])
}

/// What BO2 marks on each script anim, worked out the first time it is asked for.
fn third_infos(world: &mut World, script: &T6PlayerAnims) -> Option<Arc<Vec<third::AnimInfo>>> {
    if let Some(info) = &world.resource::<Bodies>().third_info {
        return info.clone();
    }
    let info = {
        let zm = world.resource::<Zm>();
        third_table::anim_infos(script, |name| {
            let a = zm.anims.get(name).filter(|a| a.fps > 0.0)?;
            Some((((f32::from(a.frame_total) / a.fps) * 1000.0) as i32, a.looping))
        })
        .map(std::sync::Arc::new)
    };
    diag::info!(
        Sim,
        "bo2mp bodies: BO2 third-person rules {}",
        if info.is_some() { "on" } else { "off (too many anims for BO2's value)" }
    );
    world.resource_mut::<Bodies>().third_info = Some(info.clone());
    info
}

/// One player's facts for [`advance_third`].
struct ThirdIn<'a> {
    client: u32,
    anim_type: &'a str,
    class: &'static str,
    off_type: &'a str,
    off_class: &'static str,
    offhand: u32,
    /// The weapon he is switching to: its anim type and class, and whether it
    /// is a riot shield.
    next_type: &'a str,
    next_class: &'static str,
    riotshield_next: bool,
    perk: &'static str,
    stance: u8,
    direction: &'static str,
    prev_direction: &'static str,
    position: &'static str,
    clip: i32,
    ground: bool,
    dive: u8,
    mantling: bool,
    /// He went prone or got up this tick: BO2 clears the torso anim.
    prone_change: bool,
    /// The ground's slope the way he faces (`up`, `down` or `none`).
    slope: &'static str,
    /// BO2's PM_ShouldFlinch: hit within the flinch time, view height settled,
    /// no riot shield in hand or coming up.
    flinch: bool,
}

/// BO2's PM_ShouldFlinch: his view height has settled, he holds no riot shield
/// and is not switching to one, and a hit is still inside its flinch time.
fn should_flinch(ps: &AvatarState, riot_shield: bool, shield_switch: bool) -> bool {
    ps.eye_height_now == ps.eye_height_goal as f32
        && !riot_shield
        && !shield_switch
        && (ps.hurt_length - FLINCH_MS).max(0) < ps.hurt_clock
}

/// BO2's player_dmgtimer_flinchTime (ms).
const FLINCH_MS: i32 = 500;

/// BO2's third-person rules (window 14, `weapon_t6::third`): the mantle, then each event
/// through BO2's call flags, variant pick and busy-part rules, then BO2's move type.
#[allow(clippy::too_many_arguments)]
fn advance_third(
    sim: &mut World,
    pstate: &AvatarState,
    ps_new: &mut AvatarState,
    track: &mut Track,
    script: &T6PlayerAnims,
    info: &[third::AnimInfo],
    events: &[(&'static str, bool, third::AnimCall)],
    p: &ThirdIn<'_>,
) {
    let table = third_table::ThirdTable::new(script, info);
    track.body.tick(crate::SIM_STEP_MILLIS as i32);
    if track.seed == 0 {
        track.seed = 1;
    }
    let xy = xy_speed(pstate);
    let status = third_table::movestatus_word(third::move_status(xy, false));
    let facts = |grenade: bool| T6AnimFacts {
        anim_type: if grenade { p.off_type } else { p.anim_type },
        weapon_class: if grenade { p.off_class } else { p.class },
        stance: STANCE_NAMES[usize::from(p.stance)],
        direction: p.direction,
        prev_direction: p.prev_direction,
        movestatus: status,
        weapon_position: p.position,
        next_anim_type: p.next_type,
        next_weapon_class: p.next_class,
        riotshield_next: p.riotshield_next,
        perk: p.perk,
        slope: p.slope,
        ..T6AnimFacts::default()
    };
    // BO2's Mantle_Move plays the mantle each tick before the gun's events: no
    // part timer, kept while already playing, forced over what plays.
    let mut climb_legs_only = false;
    if p.mantling
        && let Some((command, _)) = mantle_command(sim, script, pstate, &facts(false), p.client)
    {
        climb_legs_only = command.part == T6AnimPart::Legs;
        let _ = track.body.execute(
            &table.command(&command),
            [1.0; 2],
            false,
            true,
            true,
            &table,
        );
    }
    let mut timed = None;
    let mut held = None;
    let mut clear_torso = p.prone_change;
    for &(event, grenade, call) in events {
        // BO2 clears the torso after the stance event and before the gun's events.
        if clear_torso && !event.starts_with("prone_to_") {
            clear_torso = false;
            prone_clear_torso(&mut track.body, &table);
        }
        // The gun's time left for what it just started (its raise, drop or reload).
        let weapon_time = match pstate.weapon_timer {
            t if t > 0 => t,
            _ => weapon_action_ms(sim, pstate, event, p.clip).unwrap_or(0),
        };
        let gun = gun_times(pstate, weapon_time);
        // Dead players never get here: BO2's dead pm types (10 and up) are out already.
        let Some((cmd, _)) = third::script_event(&mut track.body, call, &facts(grenade), &table, gun, &mut track.seed, 0)
        else {
            continue;
        };
        let Some((part, anim)) = cmd.parts[0] else {
            continue;
        };
        if part == third::Part::Torso && third::clip_ordinal(track.body.torso_anim) == anim {
            if third::AnimTable::info(&table, anim).flags & third::anim_flags::WEAPON_TIME_SCALE != 0 {
                timed = Some(track.body.torso_duration);
            }
            // The script's grenadeAnim anims handle the offhand item: it is in his hand.
            if grenade && p.offhand != 0 && script.command_of(anim - 1).is_some_and(|c| c.grenade_anim) {
                held = Some(p.offhand);
            }
        }
        if std::env::var_os("BO2MP_BODYLOG").is_some() {
            diag::info!(
                Sim,
                "bo2mp body third: {event} ({:?}) -> {:?} on {part:?}, {} ms, weapon {weapon_time} ms",
                call.event,
                script.anims.get(usize::from(anim - 1)),
                cmd.span_ms
            );
        }
    }
    if clear_torso {
        prone_clear_torso(&mut track.body, &table);
    }
    // Fan change, not BO2 (playtest : the pull-up on smaller climbs too):
    // the script's 21 and 27 climbs are legs only, so the arms kept the gun;
    // here the torso is cleared and the climb plays on the whole body.
    if climb_legs_only {
        prone_clear_torso(&mut track.body, &table);
    }
    let conds = facts(false);
    let client = p.client;
    if p.mantling {
    } else if pstate.movement_state_bits & movement_state_bits::LADDER != 0 {
        let mt = if pstate.velocity[2] < 0.0 { third::MoveType::ClimbDown } else { third::MoveType::ClimbUp };
        let _ = third::script_move(&mut track.body, mt, false, &conds, &table, client);
    } else if !p.ground && p.dive == 0 {
        // In the air the jump's anim holds.
    } else if xy <= third::dvars::MOVE && p.dive == 0 {
        let turn = match track.turning {
            1 => 1,
            -1 => 2,
            _ => 0,
        };
        // BO2's PM_Footsteps_NotMoving: standing still and not turning, a fresh hit
        // flinches him (the script's flinch event); a turn with no anim flinches too.
        let gun = gun_times(pstate, pstate.weapon_timer);
        // BO2's prone branch passes no flinch.
        let flinch = p.flinch && pstate.waterlevel < 4 && p.stance != 2;
        if flinch && turn == 0 {
            let call = third::AnimCall::new(third::AnimEvent::Flinch, true, false);
            let _ = third::script_event(&mut track.body, call, &conds, &table, gun, &mut track.seed, 0);
        } else if third::apply_still(&mut track.body, turn, pstate.waterlevel, &conds, &table, client)
            == third::MoveType::Idle
            && turn != 0
            && flinch
        {
            let call = third::AnimCall::new(third::AnimEvent::Flinch, true, true);
            let _ = third::script_event(&mut track.body, call, &conds, &table, gun, &mut track.seed, 0);
        }
    } else {
        let m = third::MoveIn {
            stance: p.stance,
            backward: pstate.movement_state_bits & movement_state_bits::BACKWARDS_RUN != 0,
            walking: pstate.movement_state_bits & movement_state_bits::WALKING != 0 || pstate.tilt_sideways != 0.0,
            sprinting: pstate.movement_state_bits & movement_state_bits::SPRINTING != 0,
            diving: p.dive != 0,
            // On slide ground (Black Ops II's movement's own flag).
            sliding: pstate.movement_state_bits & movement_t6::PMF_SLIDING != 0,
            water_level: pstate.waterlevel,
            hurt: third::hurt(pstate.hurt_clock, pstate.hurt_length),
            xy_speed: xy,
        };
        third::apply_movement(&mut track.body, &m, &conds, &table, client);
    }
    track.holds = [track.body.lower_body_clock, track.body.upper_body_clock];
    ps_new.legs_anim = third_table::packed(track.body.legs_anim);
    let torso = third_table::packed(track.body.torso_anim);
    ps_new.torso_anim = if t6_anim_part(pstate.torso_anim) == torso {
        // Same anim, same start: keep its weapon time and offhand item.
        pstate.torso_anim
    } else {
        let mut v = torso;
        if let Some(ms) = timed {
            v = t6_with_duration(v, ms);
        }
        if let Some(w) = held {
            v = t6_with_offhand(v, w);
        }
        v
    };
}

/// BO2 going prone or getting up: the torso anim gives way at once (no time, no hold).
fn prone_clear_torso(body: &mut third::Body, table: &impl third::AnimTable) {
    let play = third::Play { anim: 0, span_ms: 0, rate: 1.0, arm_countdown: false, cont: true, force: true };
    body.play(third::Part::Torso, &play, table);
}

/// Each tick: every living player's events, then his movement animation.
/// BO2's movement started the legs' jump or land (BG_AnimScriptEvent's
/// ANIM_ET_JUMP and ANIM_ET_LAND); the next body frame plays them, on the
/// slope pitch the last command averaged.
pub(crate) fn note_body_anims(game: &mut World, peer: u32, body: movement_t6::BodyAnims) {
    if let Some(mut b) = game.get_resource_mut::<Bodies>() {
        b.t6_move = true;
        let pending = b.moved.entry(peer).or_default();
        pending.jump |= body.jump;
        pending.land |= body.land;
        pending.average_pitch = body.average_pitch;
    }
}

/// BO2's movement's own legs events: a landing, then a jump (a jump off the
/// landing comes after it).
fn moved_events(events: &mut Vec<(&'static str, bool)>, moved: movement_t6::BodyAnims) {
    if moved.land {
        events.push(("land", false));
    }
    if moved.jump {
        events.push(("jump", false));
    }
}

pub(super) fn advance(world: &mut World) {
    let Some(script) = world
        .get_resource::<Bodies>()
        .and_then(|b| b.script.clone())
    else {
        return;
    };
    if !world.resource::<Bodies>().logged {
        world.resource_mut::<Bodies>().logged = true;
        diag::info!(Sim, "bo2mp bodies: {}", script.status_line());
    }
    let clients: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    for c in clients {
        let cid = ClientId(c);
        let alive = frame(world)
            .client_meta(cid)
            .is_some_and(|m| m.lifecycle == crate::PlayerSessionStage::Alive);
        let Some(pstate) = frame(world).avatar(cid).copied() else {
            continue;
        };
        let alive = alive && pstate.movement_mode < playerstate_t6::game::MOVE_KIND_DEAD;
        let mut track = world
            .resource::<Bodies>()
            .tracks
            .get(&c)
            .cloned()
            .unwrap_or_default();
        let (t6_move, moved) = {
            let mut b = world.resource_mut::<Bodies>();
            (b.t6_move, b.moved.remove(&c))
        };
        // The slope holds until his next command.
        if let Some(m) = moved {
            track.slope_pitch = m.average_pitch;
        }
        let moved = moved.unwrap_or_default();
        let slope = t6_slope(track.slope_pitch);
        if alive {
            world.resource_mut::<Bodies>().died.remove(&c);
        }
        if !alive {
            track.present = false;
            world.resource_mut::<Bodies>().tracks.insert(c, track);
            frame(world).set_t6_hit_bones(cid, None, None);
            continue;
        }
        for hold in &mut track.holds {
            *hold = (*hold - crate::SIM_STEP_MILLIS as i32).max(0);
        }
        let clip_no = crate::client_script::ammo_clip(&frame(world), cid, pstate.weapon);
        let stance = stance_of(&pstate);
        let ground = has_ground(&pstate);
        // BO2 reads the offhand item's anim type and class for every pick while one is in use
        // (BG_GetPlayerStateWeapon: a grenade out stands and runs him with the grenade's legs),
        // and the weapon he is switching to for the next-weapon conditions.
        let (in_use, next) =
            crate::gun_brain::anim_weapons(&frame(world), cid).unwrap_or((None, pstate.weapon));
        let GunWords { anim: anim_type, class } = weapon_words(world, &script, in_use.unwrap_or(pstate.weapon));
        let GunWords { anim: next_type, class: next_class } = weapon_words(world, &script, next);
        let riotshield_next = crate::gun_brain::riotshield_switching(&frame(world), cid);
        let perk = mantle_perk(&pstate);
        // BO2's gun states, this frame's and last frame's.
        let (st, last) = (pstate.weapon_state_t6, track.wstate);
        let reloading = state::is_any_reload;
        // TODO(fact): the swim in and out as a drop and raise, as our old MW2 mirror read them.
        let dropping = state::is_dropping_or_swim_in;
        let raising = state::is_raising_or_swim_out;
        let meleeing = |s: i32| (state::MELEE_INIT..=state::MELEE_END).contains(&s);
        let priming = |s: i32| matches!(s, state::OFFHAND_INIT | state::OFFHAND_PREPARE | state::OFFHAND_HOLD | state::OFFHAND_HOLDUSE);
        let throwing = |s: i32| matches!(s, state::OFFHAND_START | state::OFFHAND);
        let mut events: Vec<(&'static str, bool)> = Vec::new();
        let mut prone_change = false;
        let stick = frame(world).old_cmd_move(cid).unwrap_or([0, 0]);
        let stick_dir = stick_direction(stick);
        if !stick_dir.is_empty() {
            track.prev_dir = stick_dir;
        }
        let dive = dive_phase(&pstate);
        let mantling = pstate.movement_state_bits & movement_state_bits::MANTLE != 0;
        // A dive or a mantle moves the stance and leaves the ground on its
        // own: their animations say so, not stance changes, jumps or landings.
        let mantle_held = track.holds[0] > 0
            && script
                .anim_name(pstate.legs_anim)
                .is_some_and(|n| n.starts_with("mp_mantle"));
        let own_motion = dive != 0 || track.dive != 0 || mantling || track.mantling || mantle_held;
        if !track.present {
            events.push(("firstraiseweapon", false));
        } else if dive == 1 && track.dive != 1 {
            events.push(("dtp_takeoff", false));
        } else if dive == 2 && track.dive != 2 {
            events.push(("dtp_land", false));
        } else if own_motion {
            // BO2's own jump and land go on through a climb (a vault's end
            // plays the jump); a dive raises neither.
            moved_events(&mut events, moved);
        } else {
            match (track.stance, stance) {
                (0, 1) => events.push(("stand_to_crouch", false)),
                (1, 0) => events.push(("crouch_to_stand", false)),
                // BO2 sends no event going prone; the torso is cleared below instead.
                (1, 2) | (0, 2) => {}
                (2, 1) => events.push(("prone_to_crouch", false)),
                // BO2 stands a sprinting toon straight up from prone with its own block.
                (2, 0) if pstate.movement_state_bits & movement_state_bits::SPRINTING != 0 => events.push(("prone_to_sprint", false)),
                (2, 0) => events.push(("prone_to_stand", false)),
                _ => {}
            }
            prone_change = (track.stance == 2) != (stance == 2);
            if t6_move {
                moved_events(&mut events, moved);
            } else {
                if track.was_grounded && !ground && pstate.velocity[2] > 0.0 {
                    events.push(("jump", false));
                }
                if !track.was_grounded && ground {
                    events.push(("land", false));
                }
            }
            // BO2's footsteps: on the ground at run speed with the stick let go, every tick (the
            // legs' busy timer keeps it from restarting).
            if ground && stick == [0, 0] && xy_speed(&pstate) >= RUN_THRESHOLD {
                events.push(("run_to_idle", false));
            }
            if dropping(st) && !dropping(last) {
                events.push(("dropweapon", false));
            }
            if raising(st) && !raising(last) {
                events.push(("raiseweapon", false));
            }
            if reloading(st) && !reloading(last) {
                events.push(("reload", false));
            }
            if meleeing(st) && !meleeing(last) {
                events.push(("meleeattack", false));
            }
            if priming(st) && !priming(last) {
                events.push(("prime_grenade", true));
            }
            if throwing(st) && !throwing(last) {
                events.push(("fireweapon", true));
            }
            if pstate.weapon == track.arm && clip_no < track.clip && !reloading(st) {
                events.push(("fireweapon", false));
            }
        }
        // Weapon events go on through a dive or a mantle.
        if own_motion && track.present {
            if priming(st) && !priming(last) {
                events.push(("prime_grenade", true));
            }
            if throwing(st) && !throwing(last) {
                events.push(("fireweapon", true));
            }
            if pstate.weapon == track.arm && clip_no < track.clip && !reloading(st) {
                events.push(("fireweapon", false));
            }
        }
        // BO2 reads the way the stick pushes; let go, the way he still slides.
        let direction = match stick_dir {
            "" => t6_move_direction(pstate.velocity, pstate.look_angles[1]),
            d => d,
        };
        // Window 14: the T6 brain's own anim script events since the last frame, and its
        // weaponstate (a planted riot shield is the script's "raised" position).
        let fed = crate::gun_brain::take_third(&mut frame(world), cid);
        let flinch = {
            let fw = frame(world);
            let riot = fw.gun_facts().get(pstate.weapon).core.gun_family_code == weapon_t6::facts::gun_family_code::RIOTSHIELD;
            should_flinch(&pstate, riot, riotshield_next)
        };
        let position = if fed.as_ref().is_some_and(|f| f.1 == weapon_t6::state::RIOTSHIELD_HOLD) {
            "raised"
        } else if pstate.movement_state_bits & movement_state_bits::ADS_INTENT != 0 {
            "ads"
        } else {
            "hip"
        };
        let was_turning = track.turning;
        let strafe = t6_anim_index(pstate.legs_anim).is_some_and(|a| script.is_strafe(a));
        let legs_off = legs_yaw_step(&mut track, &pstate, ground, strafe);
        if std::env::var_os("BO2MP_BODYLOG").is_some()
            && (track.turning != was_turning || world.resource::<Zm>().clock_millis % 1000 == 0)
        {
            diag::info!(
                Sim,
                "bo2mp body {c}: legs turn {} (view {:.0}, legs {:.0}, ground {ground}, speed {:.0})",
                track.turning,
                pstate.look_angles[1],
                track.legs_yaw,
                xy_speed(&pstate)
            );
        }
        let mut ps_new = pstate;
        let offhand = u32::try_from(pstate.secondary_hand_slot).unwrap_or(0);
        let GunWords { anim: off_type, class: off_class } = if offhand != 0 {
            weapon_words(world, &script, offhand)
        } else {
            GunWords { anim: "default".to_owned(), class: "grenade" }
        };
        let third = third_infos(world, &script);
        if let Some(info) = &third {
            let input = ThirdIn {
                client: c,
                anim_type: &anim_type,
                class,
                off_type: &off_type,
                off_class,
                offhand,
                next_type: &next_type,
                next_class,
                riotshield_next,
                perk,
                stance,
                direction,
                prev_direction: track.prev_dir,
                position,
                clip: clip_no,
                ground,
                dive,
                mantling,
                prone_change,
                slope,
                flinch,
            };
            let calls = third_table::third_calls(&events, fed.as_ref().map(|f| f.0.as_slice()));
            advance_third(world, &pstate, &mut ps_new, &mut track, &script, info, &calls, &input);
        } else {
            for (event, grenade) in events {
                let facts = T6AnimFacts {
                    anim_type: if grenade { off_type.as_str() } else { anim_type.as_str() },
                    weapon_class: if grenade { off_class } else { class },
                    stance: STANCE_NAMES[usize::from(stance)],
                    direction,
                    prev_direction: track.prev_dir,
                    movestatus: movestatus(&pstate),
                    weapon_position: position,
                    next_anim_type: &next_type,
                    next_weapon_class: next_class,
                    riotshield_next,
                    slope,
                    ..T6AnimFacts::default()
                };
                // A shield carrier's stance change only blends (see ThirdTable::event_command).
                if matches!(event, "stand_to_crouch" | "crouch_to_stand") && facts.anim_type == "riotshield" {
                    continue;
                }
                let Some(commands) = script.pick_event(event, &facts) else {
                    continue;
                };
                let Some(command) = pick(&mut world.resource_mut::<Bodies>(), commands) else {
                    continue;
                };
                // A reload, and the script's weaponTimeScale animations (raise,
                // drop), take as long as the weapon does: they end with the
                // first-person action.
                let timed = (event == "reload" || command.weapon_time_scale)
                    .then(|| weapon_action_ms(world, &pstate, event, clip_no))
                    .flatten();
                let hold = timed.or(command.play_ms).unwrap_or_else(|| anim_length_ms(world, &script, command.anim));
                apply(&mut ps_new, &mut track.holds, &command, Some(hold));
                if let Some(ms) = timed
                    && matches!(command.part, T6AnimPart::Torso | T6AnimPart::Both)
                {
                    ps_new.torso_anim = t6_with_duration(ps_new.torso_anim, ms);
                }
                // The script's grenadeAnim animations handle the offhand item
                // (a grenade thrown, a claymore planted): it is in his hand.
                if command.grenade_anim
                    && grenade
                    && offhand != 0
                    && matches!(command.part, T6AnimPart::Torso | T6AnimPart::Both)
                {
                    ps_new.torso_anim = t6_with_offhand(ps_new.torso_anim, offhand);
                }
                if std::env::var_os("BO2MP_BODYLOG").is_some() && matches!(event, "reload" | "dropweapon" | "raiseweapon") {
                    diag::info!(
                        Sim,
                        "bo2mp body {c}: {event} {:?} own {} ms, weapon {:?} ms",
                        script.anims.get(usize::from(command.anim)),
                        anim_length_ms(world, &script, command.anim),
                        timed
                    );
                }
            }
            let facts = T6AnimFacts {
                anim_type: &anim_type,
                weapon_class: class,
                stance: STANCE_NAMES[usize::from(stance)],
                direction,
                prev_direction: track.prev_dir,
                movestatus: movestatus(&pstate),
                weapon_position: position,
                next_anim_type: &next_type,
                next_weapon_class: next_class,
                riotshield_next,
                perk,
                slope,
                ..T6AnimFacts::default()
            };
            if mantling && let Some((command, left_ms)) = mantle_command(world, &script, &pstate, &facts, c) {
                if t6_anim_index(ps_new.legs_anim) != Some(command.anim) {
                    apply(&mut ps_new, &mut track.holds, &command, Some(left_ms));
                }
                // Fan change, not BO2: a legs-only climb (21, 27) plays on the
                // whole body (no torso: the body draws the legs' climb there).
                if command.part == T6AnimPart::Legs {
                    ps_new.torso_anim = 0;
                    track.holds[1] = 0;
                }
            }
            if let Some(mut mt) = movetype(&pstate) {
                if mt == "idle" && track.turning != 0 {
                    mt = if track.turning > 0 { "turnright" } else { "turnleft" };
                }
                let commands = script
                    .pick_move(mt, &facts)
                    .or_else(|| (mt != "idle").then(|| script.pick_move("idle", &facts)).flatten());
                if let Some(commands) = commands {
                    // Keep a variant already playing (a two-animation idle
                    // must not flip each tick).
                    let playing = commands
                        .iter()
                        .find(|c| t6_anim_index(ps_new.legs_anim) == Some(c.anim))
                        .copied();
                    let command = match playing {
                        Some(c) => Some(c),
                        None => pick(&mut world.resource_mut::<Bodies>(), commands),
                    };
                    if let Some(command) = command {
                        apply(&mut ps_new, &mut track.holds, &command, None);
                    }
                }
            }
        }
        ps_new.legs_anim = t6_with_legs_yaw(ps_new.legs_anim, legs_off);
        if ps_new.legs_anim != pstate.legs_anim || ps_new.torso_anim != pstate.torso_anim {
            let changed = (ps_new.legs_anim ^ pstate.legs_anim) & 0xffff != 0
                || ps_new.torso_anim != pstate.torso_anim;
            if changed && std::env::var_os("BO2MP_BODYLOG").is_some() {
                let [ox, oy, oz] = pstate.origin;
                let yaw = pstate.look_angles[1];
                diag::info!(
                    Sim,
                    "bo2mp body {c}: legs {:?} torso {:?} (timers {} {}) legs yaw {legs_off:.0} at ({:.0} {:.0} {:.0}) yaw {:.0}",
                    script.anim_name(ps_new.legs_anim),
                    script.anim_name(ps_new.torso_anim),
                    track.holds[0],
                    track.holds[1],
                    ox,
                    oy,
                    oz,
                    yaw
                );
            }
            if let Some(p) = frame(world).occupant_mut(cid) {
                p.legs_anim = ps_new.legs_anim;
                p.torso_anim = ps_new.torso_anim;
            }
        }
        let now = world.resource::<Zm>().clock_millis;
        let mut anim_start = track.anim_start;
        for (k, value) in [ps_new.legs_anim, ps_new.torso_anim].into_iter().enumerate() {
            let v = t6_anim_part(value);
            if anim_start[k].0 != v {
                anim_start[k] = (v, now);
            }
        }
        let gun_in_hand = pstate.weapon;
        track = Track {
            present: true,
            holds: track.holds,
            legs_yaw: track.legs_yaw,
            legs_swinging: track.legs_swinging,
            turning: track.turning,
            last_view: track.last_view,
            dive: dive_phase(&pstate),
            mantling: pstate.movement_state_bits & movement_state_bits::MANTLE != 0,
            stance,
            was_grounded: ground,
            arm: gun_in_hand,
            wstate: pstate.weapon_state_t6,
            clip: clip_no,
            anim_start,
            body: track.body,
            seed: track.seed,
            prev_dir: track.prev_dir,
            slope_pitch: track.slope_pitch,
        };
        // His hit boxes as his body stands now.
        let mut posed_ps = pstate;
        posed_ps.legs_anim = ps_new.legs_anim;
        posed_ps.torso_anim = ps_new.torso_anim;
        let clock = std::time::Instant::now();
        let (bones, shield_tag) = hit_bones(world, c, &posed_ps, &track, &script, now).unzip();
        frame(world).set_t6_hit_bones(cid, bones.map(Arc::new), shield_tag.flatten());
        if std::env::var_os("BO2MP_HITLOG").is_some() {
            let us = clock.elapsed().as_micros() as u64;
            let mut b = world.resource_mut::<Bodies>();
            b.hit_us = (b.hit_us.0 + us, b.hit_us.1.max(us), b.hit_us.2 + 1);
            if now % 5000 < crate::SIM_STEP_MILLIS as i64 && c == 0 {
                let (sum, max, n) = std::mem::take(&mut b.hit_us);
                diag::info!(Sim, "bo2mp hit boxes posed: {n} times, {} us average, {max} us at most", sum / n.max(1));
            }
        }
        world.resource_mut::<Bodies>().tracks.insert(c, track);
    }
}

/// The torso's animation outweighs the legs' on the bones both move (as
/// his body is drawn).
const TORSO_OVER_LEGS: f32 = 1000.0;

/// His hit boxes in his own space (feet at the origin, facing +x): BO2's
/// per-bone boxes of his body and head models (their bone info, each with
/// its hit location), posed as his body is drawn — the animations the
/// script plays on his legs and torso at their time, his aim, his legs
/// turned under him. With them, the tag his riot shield hangs on (its
/// place and axes, BO2's G_GetShieldTagMatrix) when he bears one.
fn hit_bones(
    sim: &mut World,
    c: u32,
    ps: &AvatarState,
    track: &Track,
    script: &T6PlayerAnims,
    now: i64,
) -> Option<(Vec<xmodel_runtime::HitShapeBone>, Option<xmodel_runtime::HitShapeBone>)> {
    use xmodel_runtime::{ClipPlayback, MountedModel, PosedSkeleton, DObjAnim};
    let look = sim.get_resource::<Bodies>()?.looks.get(&c)?.clone();
    if look.mesh.is_empty() {
        return None;
    }
    let mut shield_slot: Option<usize> = None;
    let caps: Vec<(Arc<xmodel_runtime::KeptModelAbility>, Option<String>)> = {
        let f = frame(sim);
        let body = f.model_capability(&look.mesh).flatten()?;
        let mut v = vec![(body, None)];
        for (m, tag_name) in &look.attached {
            if let Some(cap) = f.model_capability(m).flatten() {
                v.push((cap, Some(tag_name.clone())));
            }
        }
        // The riot shield he bears is a model attached to him like the rest
        // (in his left hand or on his back), and it has boxes of its own.
        if let Some(Held { mesh, bone }) = &look.shield
            && let Some(cap) = f.model_capability(mesh).flatten()
        {
            shield_slot = Some(v.len());
            v.push((cap, Some(bone.to_string())));
        }
        v
    };
    let models: Vec<(&xmodel_runtime::PoseDonorModel, Option<MountedModel>)> = caps
        .iter()
        .map(|(cap, tag_name)| {
            let attach = tag_name.as_ref().map(|tt| MountedModel {
                host_model: 0,
                mount_tag: if tt.is_empty() {
                    xmodel_runtime::empty_tag_attach(&caps[0].0.pose, &cap.pose)
                } else {
                    tt.clone()
                },
            });
            (&cap.pose, attach)
        })
        .collect();
    let Ok(dobj) = PosedSkeleton::build(&models) else {
        return None;
    };
    // The upper body the torso's animation drives: j_spinelower down.
    let mut upper = xmodel_runtime::PartSet::default();
    if let Some(root) = dobj.find("j_spinelower") {
        let mut on = vec![false; dobj.bones.len()];
        for i in 0..dobj.bones.len() {
            on[i] = i == root || dobj.bones[i].up.is_some_and(|p| p < i && on[p]);
            if on[i] {
                upper.set(i);
            }
        }
    } else {
        upper = dobj.full_part_set();
    }
    let playing = |value: i32, start: i64, torso: bool| {
        let name = script.anim_name(value)?;
        let clip_no = sim.resource::<Zm>().clips.get(&name.to_ascii_lowercase())?.clone();
        let duration = clip_no.duration().max(1e-3);
        let mut rate = 1.0;
        if torso && let Some(secs) = crate::body_anim_script::t6_anim_duration(value) {
            rate = (duration / secs.max(0.05)).clamp(0.2, 5.0);
        }
        let tt = (now - start).max(0) as f32 / 1000.0 * rate;
        let tt = if clip_no.looping { tt.rem_euclid(duration) } else { tt.min(duration) };
        Some((clip_no, tt))
    };
    let legs = playing(ps.legs_anim, track.anim_start[0].1, false);
    let torso = playing(ps.torso_anim, track.anim_start[1].1, true);
    let tracks: Vec<Vec<Option<usize>>> = [&legs, &torso]
        .iter()
        .map(|p| p.as_ref().map_or_else(Vec::new, |(clip_no, _)| dobj.tracks_for(clip_no)))
        .collect();
    let mut instances = Vec::with_capacity(2);
    if let Some((clip_no, tt)) = &legs {
        instances.push(ClipPlayback { clip: clip_no, tracks: &tracks[0], time: *tt, weight: 1.0, parts: None });
    }
    if let Some((clip_no, tt)) = &torso {
        instances.push(ClipPlayback {
            clip: clip_no,
            tracks: &tracks[1],
            time: *tt,
            weight: TORSO_OVER_LEGS,
            parts: Some(&upper),
        });
    }
    let pitch = ps.look_angles[0];
    let ctl = xmodel_runtime::BodyAimInput {
        look_pitch_degrees: pitch,
        lying: ps.e_flags & playerstate_t6::game::entity_flag_bits::LYING_FLAT != 0,
        crouched: ps.e_flags & playerstate_t6::game::entity_flag_bits::CROUCH_BIT != 0,
        body_lean_ratio: f32::default(),
    };
    let mut posed = dobj.pose_using_aim(&instances, &dobj.full_part_set(), glam::Mat4::IDENTITY, |rig, _, locals| {
        xmodel_runtime::pose_body_aim(rig, locals, ctl);
    });
    // His legs turned under him (turning in place, a diagonal move); on a
    // ladder all of him faces it.
    let legs_yaw = crate::body_anim_script::t6_legs_yaw(ps.legs_anim).to_radians();
    let climbing = script
        .anim_name(ps.legs_anim)
        .is_some_and(|n| n.starts_with("pb_climb") || n.starts_with("pb_riot_climb"));
    if legs_yaw.abs() > 1e-4 {
        if climbing {
            let turn = glam::Mat4::from_rotation_z(legs_yaw);
            for m in &mut posed {
                *m = turn * *m;
            }
        } else if let (Some(pelvis), Some(stabilizer)) = (dobj.find("pelvis"), dobj.find("torso_stabilizer"))
            && let Some(pivot) = posed.get(pelvis).map(|m| m.w_axis.truncate())
        {
            let turn = glam::Mat4::from_translation(pivot)
                * glam::Mat4::from_rotation_z(legs_yaw)
                * glam::Mat4::from_translation(-pivot);
            let mut legs = vec![false; dobj.bones.len().min(posed.len())];
            for i in 0..legs.len() {
                legs[i] = i == pelvis || (i != stabilizer && dobj.bones[i].up.is_some_and(|p| p < i && legs[p]));
                if legs[i] {
                    posed[i] = turn * posed[i];
                }
            }
        }
    }
    let mut out = Vec::new();
    // The shield model's boxes as its own file classes them (for the log).
    let mut shield_classes: Vec<u8> = Vec::new();
    for (index, (slot, (cap, _))) in dobj.models.iter().zip(&caps).enumerate() {
        for bone_ix in 0..slot.joint_total {
            let Some(Some(bc)) = cap.joint_hit_volume.get(bone_ix) else {
                continue;
            };
            let Some(m) = posed.get(slot.first_joint + bone_ix) else {
                continue;
            };
            let Some(mut b) = xmodel_runtime::collision_bone_from_local_box(
                u16::try_from(slot.first_joint + bone_ix).ok()?,
                bc.midpoint,
                bc.half_ext,
                *m,
            ) else {
                continue;
            };
            b.body_part_category = bc.body_part_category;
            // A bullet that meets the shield's box is a hit on the shield
            // ("riotshield", the highest-priority part in BO2's bullet
            // table), whatever the box's own class number.
            if shield_slot == Some(index) {
                shield_classes.push(bc.body_part_category);
                b.body_part_category = crate::shot_trace::HITLOC_SHIELD;
            }
            out.push(b);
        }
    }
    if std::env::var_os("BO2MP_SHIELDLOG").is_some()
        && look.shield.is_some()
        && now % 5000 < crate::SIM_STEP_MILLIS as i64
    {
        diag::info!(
            Sim,
            "bo2mp shield boxes {c}: {} boxes on the shield model {:?} at {:?} (model loaded: {}, its own classes {shield_classes:?}); the cone fallback is {}",
            shield_classes.len(),
            look.shield.as_ref().map(|s| s.mesh.as_str()),
            look.shield.as_ref().map(|s| s.bone),
            shield_slot.is_some(),
            if shield_classes.is_empty() { "in use" } else { "off" }
        );
    }
    if std::env::var_os("BO2MP_HITLOG").is_some() && now % 5000 < crate::SIM_STEP_MILLIS as i64 {
        let mut kinds: Vec<u8> = out.iter().map(|b| b.body_part_category).collect();
        kinds.sort_unstable();
        kinds.dedup();
        diag::info!(
            Sim,
            "bo2mp hit boxes {c}: {} boxes on {} ({} models), hit locations {kinds:?}; legs {:?} ({}) torso {:?} ({}); boxes {:?}",
            out.len(),
            look.mesh,
            caps.len(),
            script.anim_name(ps.legs_anim),
            legs.is_some(),
            script.anim_name(ps.torso_anim),
            torso.is_some(),
            out.iter()
                .map(|b| (b.body_part_category, b.center.map(|v| v.round()), b.half_ext.map(|v| v.round())))
                .collect::<Vec<_>>()
        );
    }
    let shield_tag = look
        .shield
        .as_ref()
        .and_then(|s| dobj.find(s.bone))
        .and_then(|i| posed.get(i))
        .and_then(|m| xmodel_runtime::collision_bone_from_local_box(0, [0.0; 3], [1.0; 3], *m));
    (!out.is_empty()).then_some((out, shield_tag))
}

/// A lethal hit: his death animation, as the script's DEATH event picks it
/// from how he died (`dmgType`: explosive, headshot, melee, shotgun) and
/// from where (`dmgDirection`: the side the hit came from), his stance and
/// his movement. The engine's corpse copies it.
pub(super) fn death(game: &mut World, cid: ClientId, means: &str, aim: Option<[f32; 3]>, gun: &str) {
    let Some(script) = game
        .get_resource::<Bodies>()
        .and_then(|b| b.script.clone())
    else {
        return;
    };
    let Some(pstate) = frame(game).avatar(cid).copied() else {
        return;
    };
    let w = super::weapon(game, gun).unwrap_or(0);
    let GunWords { anim: anim_type, class } = weapon_words(game, &script, w);
    let means = means.to_ascii_uppercase();
    let dmg_type = if means == "MOD_HEAD_SHOT" {
        "headshot"
    } else if means.starts_with("MOD_MELEE") || means == "MOD_BAYONET" {
        if anim_type == "riotshield" { "meleebash" } else { "melee" }
    } else if means.contains("GRENADE") || means.contains("EXPLOSIVE") || means.contains("PROJECTILE") {
        "explosive"
    } else if class == "spread" {
        "normal_shotgun"
    } else {
        "normal"
    };
    // The side the hit came from: the way back along its direction, against
    // the way he faced.
    let dmg_direction = aim
        .filter(|d| d[0] * d[0] + d[1] * d[1] > 1e-6)
        .map_or("front", |d| {
            let from = (-d[1]).atan2(-d[0]).to_degrees();
            let mut rel = from - pstate.look_angles[1];
            while rel > 180.0 {
                rel -= 360.0;
            }
            while rel < -180.0 {
                rel += 360.0;
            }
            if rel.abs() <= 45.0 {
                "front"
            } else if rel.abs() >= 135.0 {
                "back"
            } else if rel > 0.0 {
                "left"
            } else {
                "right"
            }
        });
    let stance = STANCE_NAMES[usize::from(stance_of(&pstate))];
    let facts = T6AnimFacts {
        anim_type: &anim_type,
        weapon_class: class,
        stance,
        direction: t6_move_direction(pstate.velocity, pstate.look_angles[1]),
        movestatus: movestatus(&pstate),
        weapon_position: "hip",
        dmg_type,
        dmg_direction,
        ..T6AnimFacts::default()
    };
    let Some(commands) = script.pick_event("death", &facts) else {
        return;
    };
    let Some(command) = pick(&mut game.resource_mut::<Bodies>(), commands) else {
        return;
    };
    let both = T6AnimCommand {
        part: T6AnimPart::Both,
        ..command
    };
    let hold = anim_length_ms(game, &script, command.anim);
    if std::env::var_os("BO2MP_BODYLOG").is_some() {
        diag::info!(
            Sim,
            "bo2mp body {}: death {:?} ({dmg_type} from the {dmg_direction}, {stance})",
            cid.0,
            script.anims.get(usize::from(command.anim))
        );
    }
    if let Some(p) = frame(game).occupant_mut(cid) {
        apply(p, &mut [0, 0], &both, Some(hold));
    }
    game.resource_mut::<Bodies>().died.insert(cid.0);
}

/// BO2's player bodies: four, shared by everyone (G_GetFreePlayerCorpseIndex).
const BO2_PLAYER_CORPSES: usize = 4;

/// The slot BO2 gives a new body: the first free one of the four, else the
/// one whose body lies farthest from the first player (entity 0), which goes.
/// A player state's `body_index` while his view is not on a body.
const NO_CORPSE: i32 = -1;

fn bo2_corpse_slot(pool: &crate::PlayerCorpsePool, first: [f32; 3]) -> usize {
    let slots = &pool.slots[..BO2_PLAYER_CORPSES];
    if let Some(i) = slots.iter().position(|s| s.entnum == -1) {
        return i;
    }
    let mut best = 0;
    let mut farthest = -1.0f32;
    for (i, s) in slots.iter().enumerate() {
        let d: f32 = (0..3).map(|k| (s.origin[k] - first[k]).powi(2)).sum();
        if d > farthest {
            best = i;
            farthest = d;
        }
    }
    best
}

/// The body in `slot` goes to make room for a new one: its script entity
/// too (BO2 frees the corpse entity itself), and a dead owner's view stops
/// following it.
fn drop_corpse_in(vm: &mut Vm<World>, game: &mut World, slot_no: usize, gone: crate::PlayerCorpseSlot) {
    let n = game.get_resource::<Bodies>().and_then(|b| {
        b.corpse_slots
            .iter()
            .find(|(_, v)| usize::from(v.0) == slot_no && v.1 == gone.entnum)
            .map(|(n, _)| *n)
    });
    if let Some(n) = n {
        if let Some(mut b) = game.get_resource_mut::<Bodies>() {
            b.corpse_slots.remove(&n);
            b.corpse_anims.remove(&n);
            b.corpses.retain(|_, v| *v != n);
        }
        let obj = game.resource_mut::<Zm>().ents.remove(&n).and_then(|e| e.obj);
        if let Some(o) = obj {
            vm.free_object(game, o);
        }
    }
    let mut f = frame(game);
    f.corpses_mut().slots[slot_no] = crate::PlayerCorpseSlot::default();
    if let Some(ps) = f.occupant_mut(gone.victim)
        && ps.body_index == i32::try_from(slot_no).unwrap_or(-1)
    {
        ps.body_index = NO_CORPSE;
    }
    diag::info!(
        Sim,
        "bo2mp corpse: slot {slot_no} taken for a new body, farthest from player 0 (body {n:?}, engine corpse {})",
        gone.entnum
    );
}

/// `cloneplayer`: the engine's corpse of him (his death animation, where he
/// fell) and an entity the scripts keep for it (`self.body`). BO2 keeps four
/// bodies shared by everyone: the new one takes a free slot, else the body
/// farthest from the first player goes, entity and all. His earlier bodies
/// stay until then.
pub(super) fn clone_player(vm: &mut Vm<World>, sim: &mut World, peer: u32) -> ScriptValue {
    let id = ClientId(peer);
    let Some(pstate) = frame(sim).avatar(id).copied() else {
        return ScriptValue::Undefined;
    };
    // A death no hit announced (a fall, a suicide): his death animation
    // first, never a standing corpse.
    if !sim
        .get_resource::<Bodies>()
        .is_some_and(|b| b.died.contains(&peer))
    {
        death(sim, id, "MOD_UNKNOWN", None, "none");
    }
    let tick = super::tick(sim);
    let first = frame(sim).avatar(ClientId(0)).map_or([0.0; 3], |p| p.origin);
    let slot = bo2_corpse_slot(frame(sim).body_pool(), first);
    let gone = frame(sim).body_pool().slots[slot];
    if gone.entnum != -1 {
        drop_corpse_in(vm, sim, slot, gone);
    }
    let lap = sim.get_resource_mut::<Bodies>().is_none_or(|mut b| {
        b.corpse_laps[slot] = !b.corpse_laps[slot];
        b.corpse_laps[slot]
    });
    let corpse_entnum = playerstate_t6::game::BODY_ENTITY_FIRST_SLOT + slot as i32;
    let bit = if lap { crate::bodies::CLONE_NEW_BODY_FLAG } else { 0 };
    let stamp_ms = crate::bodies::level_time_ms(tick);
    let mut f = frame(sim);
    let Some(now) = f.avatar(id).copied() else {
        return ScriptValue::Undefined;
    };
    f.corpses_mut().release_entity_slot(corpse_entnum);
    let slot =
        crate::bodies::occupy_player_clone_at(&mut f, id, &now, stamp_ms, slot as u8, corpse_entnum, bit);
    // His view follows his body, as clone_corpse marks him.
    if let Some(p) = f.occupant_mut(id) {
        p.body_index = i32::from(slot);
        p.e_flags |= playerstate_t6::game::entity_flag_bits::DEAD;
    }
    drop(f);
    let n = sim.resource_mut::<Zm>().alloc_entnum();
    let obj = vm.alloc_object(ObjKind::Entity(n));
    sim.resource_mut::<Zm>().ents.insert(
        n,
        Ent {
            obj: Some(obj),
            class_tag: "player_corpse".to_owned(),
            origin: pstate.origin,
            rot: [0.0, pstate.look_angles[1], 0.0],
            ..Default::default()
        },
    );
    // The death animation the corpse plays (his legs' as he fell).
    let legs = frame(sim).avatar(id).map(|p| p.legs_anim);
    let anim = legs.and_then(|legs| {
        sim
            .get_resource::<Bodies>()?
            .script
            .as_ref()?
            .anim_name(legs)
            .map(str::to_owned)
    });
    if let Some(mut b) = sim.get_resource_mut::<Bodies>() {
        b.corpses.insert(peer, n);
        b.corpse_slots.insert(n, (slot, corpse_entnum));
        if let Some(anim) = anim {
            b.corpse_anims.insert(n, anim);
        }
    }
    ScriptValue::Object(obj)
}

/// A body entity's engine corpse slot, while that corpse is still its own
/// (the pool hands a slot on to a newer corpse).
fn corpse_slot(vm: &Vm<World>, game: &mut World, st: &ScriptValue) -> Option<usize> {
    let n = super::entnum(vm, st)?;
    let (slot, entnum) = *game.get_resource::<Bodies>()?.corpse_slots.get(&n)?;
    let slot = usize::from(slot);
    let pool = frame(game);
    let it = pool.body_pool().slots.get(slot)?;
    (it.occupied && it.entnum == entnum).then_some(slot)
}

/// `delete` on a body entity (`cloneplayer`): its engine corpse goes with it,
/// as BO2 frees the corpse entity itself (the map scripts' `destroy_corpses`
/// clearing a train, crane, cable car or gondola). Leaves the slot alone once
/// the pool has handed it on to a newer body.
pub(super) fn free_body(world: &mut World, n: u32) {
    let Some(mut b) = world.get_resource_mut::<Bodies>() else {
        return;
    };
    let Some((slot, entnum)) = b.corpse_slots.remove(&n) else {
        return;
    };
    b.corpse_anims.remove(&n);
    let owner = b.corpses.iter().find(|(_, v)| **v == n).map(|(c, _)| *c);
    if let Some(c) = owner {
        b.corpses.remove(&c);
    }
    let slot = usize::from(slot);
    let mut pool = frame(world);
    let mine = pool.body_pool().slots.get(slot).is_some_and(|it| {
        it.occupied && it.entnum == entnum && owner.is_none_or(|c| it.victim.0 == c)
    });
    if !mine {
        return;
    }
    pool.corpses_mut().slots[slot] = crate::PlayerCorpseSlot::default();
    if let Some(c) = owner
        && let Some(ps) = pool.occupant_mut(ClientId(c))
        && ps.body_index == i32::try_from(slot).unwrap_or(-1)
    {
        ps.body_index = NO_CORPSE;
    }
    diag::info!(Sim, "bo2mp corpse: body {n} deleted, engine corpse {entnum} (slot {slot}) freed");
}

/// The engine's player corpses, for the menu's guest weapons
/// (`mp_specials`): (slot, entity number, whose, whether it is his newest).
pub(super) fn corpses_seen(game: &mut World) -> Vec<(usize, i32, u32, bool)> {
    let newest: Vec<(u8, i32)> = game
        .get_resource::<Bodies>()
        .map(|b| b.corpses.values().filter_map(|n| b.corpse_slots.get(n).copied()).collect())
        .unwrap_or_default();
    let pool = frame(game);
    pool.body_pool()
        .slots
        .iter()
        .enumerate()
        .where_value(|it| it.occupied && it.entnum >= 0)
        .map(|(slot_no, it)| (slot_no, it.entnum, it.victim.0, newest.iter().any(|(st, en)| usize::from(*st) == slot_no && *en == it.entnum)))
        .collect()
}

/// A guest weapon's hit on a body: it goes limp, thrown by `launch`
/// (`launchragdoll`'s units) if given. Whether it was there.
pub(super) fn limp_corpse(game: &mut World, slot_no: usize, ent_no: i32, launch: Option<[f32; 3]>) -> bool {
    let there = frame(game).body_pool().slots.get(slot_no).is_some_and(|it| it.occupied && it.entnum == ent_no);
    if there {
        go_limp(game, slot_no, launch);
    }
    there
}

/// Ragdoll trajectories (BO2's numbers): 12 the body going limp where it
/// lies, 13 limp and thrown (a `launchragdoll` when its duration is 1), 14
/// come to rest.
const TR_RAGDOLL: u8 = entity_t6::path_kind::LIMP;
const TR_RAGDOLL_GRAVITY: u8 = entity_t6::path_kind::LIMP_FALLING;

/// `startragdoll` / `launchragdoll`: the body goes limp (the clients'
/// ragdoll takes it from its death animation), thrown by `launch` if given.
fn go_limp(game: &mut World, slot_no: usize, launch: Option<[f32; 3]>) {
    let time = crate::bodies::level_time_ms(super::tick(game));
    let mut pool = frame(game);
    let it = &mut pool.corpses_mut().slots[slot_no];
    let limp = (TR_RAGDOLL..=entity_t6::path_kind::LIMP_LAST).contains(&it.tr_type);
    match launch {
        Some(v) => {
            it.tr_type = TR_RAGDOLL_GRAVITY;
            it.tr_delta = v;
            it.tr_duration = 1;
            it.falling = true;
        }
        None if limp => return,
        // Still falling: it keeps falling, limp.
        None if it.falling => {
            it.tr_type = TR_RAGDOLL_GRAVITY;
            it.tr_duration = 0;
        }
        None => {
            it.tr_type = TR_RAGDOLL;
            it.tr_delta = [0.0; 3];
            it.tr_duration = 0;
        }
    }
    it.tr_base = it.origin;
    it.tr_time = time;
    if std::env::var_os("BO2MP_BODYDRAWLOG").is_some() {
        diag::info!(World, "bo2mp ragdoll: corpse slot {slot_no} limp, launch {launch:?}");
    }
}

/// `physicsexplosionsphere`'s kicks, as `launchragdoll` vectors: BO2's
/// launch gives each bone ragdoll_bullet_force 0.5 x the vector, so a kick
/// of 18 x scale is the vector 36 x scale.
fn explosion_kicks(game: &mut World, centre: [f32; 3], outer: f32, inner: f32, scale: f32) -> Vec<(usize, [f32; 3])> {
    const EXPLODE_FORCE: f32 = 18.0;
    const UP_BIAS: f32 = 0.8;
    const BULLET_FORCE: f32 = 0.5;
    const PELVIS_HEIGHT: f32 = 40.0;
    const NEAREST: usize = 4;
    let (outer_sq, inner_sq) = (outer * outer, inner * inner);
    if outer <= 0.0 {
        return Vec::new();
    }
    let pool = frame(game);
    let mut near: Vec<(f32, usize, [f32; 3])> = pool
        .body_pool()
        .slots
        .iter()
        .enumerate()
        .filter(|(_, it)| it.occupied && (TR_RAGDOLL..=entity_t6::path_kind::LIMP_LAST).contains(&it.tr_type))
        .map(|(slot, it)| {
            let d = [it.origin[0] - centre[0], it.origin[1] - centre[1], it.origin[2] + PELVIS_HEIGHT - centre[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2], slot, d)
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter()
        .take(NEAREST)
        .filter(|(dist_sq, ..)| *dist_sq < outer_sq)
        .map(|(dist_sq, slot, d)| {
            let falloff = if dist_sq > inner_sq { (outer_sq - dist_sq) / (outer_sq - inner_sq) } else { 1.0 };
            let len = dist_sq.sqrt();
            let mut dir = if len > 0.0 { [d[0] / len, d[1] / len, d[2] / len] } else { [0.0; 3] };
            dir[2] += UP_BIAS;
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let speed = EXPLODE_FORCE * scale * falloff / BULLET_FORCE / len;
            (slot, [dir[0] * speed, dir[1] * speed, dir[2] * speed])
        })
        .collect()
}

/// An animation value's name (a `%anim` reference or a name string).
fn anim_name(vm: &Vm<World>, v: &ScriptValue) -> Option<String> {
    match v {
        ScriptValue::Anim(id) => vm
            .program
            .anims
            .get(*id as usize)
            .map(|(_, a)| vm.strings.get(*a).to_ascii_lowercase()),
        ScriptValue::Str(_) | ScriptValue::IStr(_) => Some(vm.to_text(v).to_ascii_lowercase()),
        _ => None,
    }
}

/// The animation natives BO2's death path asks of a body
/// (`_globallogic_player::delaystartragdoll`): the corpse's death
/// animation, whether it is a ragdoll (never: there is no ragdoll), an
/// animation's notetracks and their times (fractions of it), its length in
/// seconds, from the animations the zones hold.
pub(super) fn bind(vm: &mut Vm<World>) {
    // BO2's riot shield on the body (maps/mp/_riotshield calls this when his
    // weapon changes): the script's carried model (level.carriedshieldmodel)
    // in his left hand while the shield is his weapon, its stowed model
    // (level.stowedshieldmodel) on his back while he only has it.
    vm.bind("refreshshieldattachment", true, |vm, world, s, _| {
        let id = super::client(vm, world, s)?;
        let level_text = |vm: &mut Vm<World>, name: &str| {
            let f = vm.intern(name);
            match vm.raw_field(vm.level, f) {
                ScriptValue::Undefined => String::new(),
                v => vm.to_text(&v),
            }
        };
        let carried = level_text(vm, "carriedshieldmodel");
        let stowed = level_text(vm, "stowedshieldmodel");
        let Some(script) = world.get_resource::<Bodies>().and_then(|b| b.script.clone()) else {
            return Ok(ScriptValue::Undefined);
        };
        let current = frame(world).avatar(id).map_or(0, |ps| ps.weapon);
        let weapons = crate::client_script::weapons(&frame(world), id, crate::client_script::WeaponList::All);
        let mut shield_of = |w: u32| w != 0 && weapon_words(world, &script, w).anim == "riotshield";
        let held = shield_of(current);
        let has = held || weapons.into_iter().any(&mut shield_of);
        let shield = if held && !carried.is_empty() {
            Some(Held { mesh: carried, bone: "tag_weapon_left" })
        } else if has && !stowed.is_empty() {
            Some(Held { mesh: stowed, bone: STOWED_TAG })
        } else {
            None
        };
        set_shield(world, id, shield);
        Ok(ScriptValue::Undefined)
    });
    // detachshieldmodel(model, tag): BO2's death path takes the carried
    // shield off his hand.
    vm.bind("detachshieldmodel", true, |vm, world, s, lhs| {
        let id = super::client(vm, world, s)?;
        let model = super::text(vm, lhs, 0);
        let on = world
            .get_resource::<Bodies>()
            .and_then(|b| b.looks.get(&id.0))
            .and_then(|l| l.shield.as_ref())
            .is_some_and(|s| s.mesh.eq_ignore_ascii_case(&model));
        if on {
            set_shield(world, id, None);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("getcorpseanim", true, |vm, world, s, _| {
        let name = super::entnum(vm, s).and_then(|n| {
            world
                .get_resource::<Bodies>()
                .and_then(|b| b.corpse_anims.get(&n).cloned())
        });
        Ok(name.map_or(ScriptValue::Undefined, |n| vm.string(&n)))
    });
    vm.bind("isragdoll", true, |vm, world, s, _| {
        let limp = corpse_slot(vm, world, s).is_some_and(|slot| {
            (TR_RAGDOLL..=12).contains(&frame(world).body_pool().slots[slot].tr_type)
        });
        Ok(ScriptValue::Int(i32::from(limp)))
    });
    // On lhs body the engine keeps (lhs corpse); anything else stays as it is.
    vm.bind("startragdoll", true, |vm, world, s, _| {
        if let Some(slot) = corpse_slot(vm, world, s) {
            go_limp(world, slot, None);
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("launchragdoll", true, |vm, world, s, lhs| {
        if let Some(slot) = corpse_slot(vm, world, s) {
            go_limp(world, slot, super::vec3(lhs, 0).ok());
        }
        Ok(ScriptValue::Undefined)
    });
    // BO2's Scr_PhysicsExplosionSphere (origin, outer radius, inner radius,
    // scale): an event the client turns into Ragdoll_ExplosionEvent, a kick
    // of ragdoll_explode_force 18 x scale over the bones, full inside the
    // inner radius and falling off by distance squared to 0 at the outer,
    // out from the centre with ragdoll_explode_upbias 0.8 added to its up,
    // on the 4 nearest limp bodies. Oldschool's death push (_globallogic_
    // player) is its main use. bo2mp: one kick per body, at its pelvis
    // (BO2's own torso_lower height, 40, gethitlocheight); BO2 kicks each
    // bone. A body already thrown keeps that throw.
    vm.bind("physicsexplosionsphere", false, |_, world, _, lhs| {
        let centre = super::vec3(lhs, 0)?;
        let outer = super::num(lhs, 1)?;
        let inner = super::num(lhs, 2)?.min(outer);
        let scale = super::num(lhs, 3)?;
        for (slot, kick) in explosion_kicks(world, centre, outer, inner, scale) {
            go_limp(world, slot, Some(kick));
        }
        Ok(ScriptValue::Undefined)
    });
    vm.bind("animhasnotetrack", false, |vm, world, _, lhs| {
        let Some(anim) = anim_name(vm, super::arg(lhs, 0)) else {
            return Ok(ScriptValue::Int(0));
        };
        let note = vm.to_text(super::arg(lhs, 1));
        let has = world
            .resource::<Zm>()
            .anims
            .get(&anim)
            .is_some_and(|x| x.notifies.iter().any(|(n, _)| n.eq_ignore_ascii_case(&note)));
        Ok(ScriptValue::bool(has))
    });
    vm.bind("getnotetracktimes", false, |vm, world, _, lhs| {
        let note = vm.to_text(super::arg(lhs, 1));
        let zm = world.resource::<Zm>();
        let times: Vec<ScriptValue> = anim_name(vm, super::arg(lhs, 0))
            .and_then(|anim| zm.anims.get(&anim))
            .map(|x| {
                x.notifies
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case(&note))
                    .map(|(_, t)| ScriptValue::Float(*t))
                    .collect()
            })
            .unwrap_or_default();
        Ok(super::list(times))
    });
    vm.bind("getanimlength", false, |vm, world, _, lhs| {
        let len = anim_name(vm, super::arg(lhs, 0))
            .and_then(|anim| {
                let x = world.resource::<Zm>().anims.get(&anim)?.clone();
                (x.fps > 0.0).then(|| f32::from(x.frame_total) / x.fps)
            })
            .unwrap_or(0.0);
        Ok(ScriptValue::Float(len))
    });
}

#[cfg(test)]
mod corpse_slot_tests {
    use super::{BO2_PLAYER_CORPSES, bo2_corpse_slot};
    use crate::{PlayerCorpsePool, PlayerCorpseSlot};

    fn body(ent_no: i32, x: f32) -> PlayerCorpseSlot {
        let mut slot_no = PlayerCorpseSlot::default();
        slot_no.occupied = true;
        slot_no.entnum = ent_no;
        slot_no.origin = [x, 0.0, 0.0];
        slot_no
    }

    /// A new body takes a free slot of BO2's four, never one past them.
    #[test]
    fn a_new_body_takes_a_free_slot_of_bo2s_four() {
        let mut pool = PlayerCorpsePool::default();
        assert_eq!(bo2_corpse_slot(&pool, [0.0; 3]), 0);
        pool.slots[0] = body(18, 10.0);
        pool.slots[2] = body(20, 10.0);
        assert_eq!(bo2_corpse_slot(&pool, [0.0; 3]), 1);
    }

    /// All four taken: the body farthest from the first player goes, wherever
    /// the new one died (the four after them stay unused).
    #[test]
    fn with_four_bodies_the_one_farthest_from_the_first_player_goes() {
        let mut pool = PlayerCorpsePool::default();
        for (i, x) in [100.0, -900.0, 300.0, 50.0].into_iter().enumerate() {
            pool.slots[i] = body(18 + i as i32, x);
        }
        assert_eq!(bo2_corpse_slot(&pool, [0.0; 3]), 1);
        assert_eq!(bo2_corpse_slot(&pool, [-800.0, 0.0, 0.0]), 2);
        assert!(bo2_corpse_slot(&pool, [5000.0, 0.0, 0.0]) < BO2_PLAYER_CORPSES);
    }
}

#[cfg(test)]
mod stick_tests {
    use super::stick_direction;

    #[test]
    fn the_stick_reads_as_bo2s_direction() {
        assert_eq!(stick_direction([0, 0]), "");
        assert_eq!(stick_direction([127, 0]), "forward");
        // A diagonal is past BO2's 0.5 cosine: still forward / backward.
        assert_eq!(stick_direction([127, 127]), "forward");
        assert_eq!(stick_direction([-127, -127]), "backward");
        assert_eq!(stick_direction([0, 127]), "right");
        assert_eq!(stick_direction([0, -127]), "left");
        assert_eq!(stick_direction([40, -127]), "left");
    }
}

#[cfg(test)]
mod still_tests {
    use super::{AvatarState, Track, legs_yaw_step, should_flinch};

    fn settled() -> AvatarState {
        let mut ps = AvatarState::ZERO;
        let (eye_goal, eye_now) = (60, 60.0);
        ps.eye_height_goal = eye_goal;
        ps.eye_height_now = eye_now;
        ps
    }

    #[test]
    fn a_fresh_hit_flinches_him_standing_still() {
        let mut ps = settled();
        // No hit yet: no flinch.
        assert!(!should_flinch(&ps, false, false));
        // A hit 1000 ms long, 600 ms left: past the first 500 ms window.
        ps.hurt_length = 1000;
        ps.hurt_clock = 600;
        assert!(should_flinch(&ps, false, false));
        ps.hurt_clock = 500;
        assert!(!should_flinch(&ps, false, false));
        // A short hit (under 500 ms) flinches while any of it is left.
        ps.hurt_length = 300;
        ps.hurt_clock = 1;
        assert!(should_flinch(&ps, false, false));
    }

    #[test]
    fn no_flinch_with_a_riot_shield_or_a_moving_view_height() {
        let mut ps = settled();
        ps.hurt_length = 1000;
        ps.hurt_clock = 900;
        assert!(!should_flinch(&ps, true, false));
        assert!(!should_flinch(&ps, false, true));
        ps.eye_height_now = 50.0;
        assert!(!should_flinch(&ps, false, false));
    }

    #[test]
    fn running_straight_the_legs_hold_through_a_wobbling_view() {
        let mut track = Track {
            present: true,
            ..Track::default()
        };
        let mut ps = AvatarState::ZERO;
        ps.velocity = [250.0, 0.0, 0.0];
        // BO2 (BG_SwingAngles, bg_legYawTolerance 40): the legs keep their yaw
        // while the view and the move wobble a few degrees either way.
        for (view, heading) in [(0.0, 0), (6.0, -6), (-6.0, 8), (4.0, -10), (-5.0, 5)] {
            ps.look_angles[1] = view;
            ps.move_heading = heading;
            legs_yaw_step(&mut track, &ps, true, false);
            assert_eq!(track.legs_yaw, 0.0);
        }
    }

    #[test]
    fn a_diagonal_run_eases_the_legs_round_to_it() {
        let mut track = Track {
            present: true,
            ..Track::default()
        };
        let mut ps = AvatarState::ZERO;
        ps.velocity = [200.0, 200.0, 0.0];
        ps.move_heading = 45;
        let mut last = 0.0;
        for _ in 0..40 {
            legs_yaw_step(&mut track, &ps, true, false);
            // Each step at most max(0.5, gap * 0.05) * frame * bg_swingSpeed.
            let gap: f32 = 45.0 - last;
            let most = (gap * 0.05).max(0.5) * crate::SIM_STEP_MILLIS as f32 * 0.15;
            assert!(track.legs_yaw - last <= most + 1e-3);
            assert!(track.legs_yaw <= 45.0);
            last = track.legs_yaw;
        }
        assert!((track.legs_yaw - 45.0).abs() < 1e-3);
        // A strafe animation's legs swing back to the view.
        for _ in 0..40 {
            legs_yaw_step(&mut track, &ps, true, true);
        }
        assert!(track.legs_yaw.abs() < 1e-3 || (track.legs_yaw - 360.0).abs() < 1e-3);
    }

    #[test]
    fn a_turn_in_place_follows_the_way_the_view_moves() {
        let mut track = Track {
            present: true,
            ..Track::default()
        };
        let mut ps = AvatarState::ZERO;
        // He turns right past 45 degrees: the legs turn right.
        ps.look_angles[1] = -50.0;
        legs_yaw_step(&mut track, &ps, true, false);
        assert_eq!(track.turning, 1);
        // Mid-turn he swings the view back left: BO2 plays the left turn.
        ps.look_angles[1] = -40.0;
        legs_yaw_step(&mut track, &ps, true, false);
        assert_eq!(track.turning, -1);
        // The view holds still: the turn keeps its way.
        legs_yaw_step(&mut track, &ps, true, false);
        assert_eq!(track.turning, -1);
    }
}

#[cfg(test)]
mod moved_tests {
    use super::{Bodies, moved_events, note_body_anims};
    use bevy_ecs::prelude::World;
    use movement_t6::BodyAnims;

    #[test]
    fn bo2s_jump_and_land_wait_for_the_next_body_frame() {
        let mut world = World::new();
        // No bodies (the client's prediction): nothing kept.
        let jump = BodyAnims { jump: true, ..BodyAnims::default() };
        note_body_anims(&mut world, 3, jump);
        world.insert_resource(Bodies::new(None));
        assert!(!world.resource::<Bodies>().t6_move);
        // Two commands in one frame: both kept, on the last one's slope.
        note_body_anims(&mut world, 3, BodyAnims { land: true, average_pitch: 12.0, ..BodyAnims::default() });
        note_body_anims(&mut world, 3, BodyAnims { average_pitch: -4.0, ..jump });
        note_body_anims(&mut world, 4, BodyAnims::default());
        let b = world.resource::<Bodies>();
        assert!(b.t6_move);
        assert_eq!(b.moved[&3], BodyAnims { jump: true, land: true, average_pitch: -4.0 });
        assert_eq!(b.moved[&4], BodyAnims::default());
        // A landing, then the jump off it.
        let mut first = Vec::new();
        moved_events(&mut first, b.moved[&3]);
        assert_eq!(first, [("land", false), ("jump", false)]);
        let mut second = Vec::new();
        moved_events(&mut second, b.moved[&4]);
        assert!(second.is_empty());
    }
}
