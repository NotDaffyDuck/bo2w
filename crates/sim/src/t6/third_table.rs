//! Window 14 (gun brain): BO2's third-person play rules (`weapon_t6::third`) run on sim's
//! parsed `playeranim.script`. A third-person anim value is the script's anim index + 1
//! (0 = none), the same number sim packs; only the restart bit differs (BO2 0x400, ours
//! [`T6_ANIM_TOGGLE`]).

use std::marker::PhantomData;

use weapon_t6::events::AnimScript;
use weapon_t6::third::{
    self, AnimCall, AnimEvent, AnimInfo, AnimTable, Command, MoveType, Part, RESTART_BIT,
    anim_flags,
};

use crate::t6_playeranim::{
    T6_ANIM_INDEX_MASK, T6_ANIM_TOGGLE, T6AnimCommand, T6AnimFacts, T6AnimPart, T6PlayerAnims,
};

/// What BO2 marks on each anim when the script loads, by the blocks that name it.
/// `anim(name)` gives an anim's own length (ms) and whether it loops. `None` when the
/// script has more anims than BO2's 10-bit value holds.
pub(super) fn anim_infos(
    script: &T6PlayerAnims,
    anim: impl Fn(&str) -> Option<(i32, bool)>,
) -> Option<Vec<AnimInfo>> {
    if script.anims.len() >= usize::from(RESTART_BIT) {
        return None;
    }
    let mut info = vec![AnimInfo::default(); script.anims.len() + 1];
    for (i, name) in script.anims.iter().enumerate() {
        let (len_ms, looped) = anim(name).unwrap_or((500, false));
        let slot = &mut info[i + 1];
        slot.len_ms = len_ms;
        slot.moves = script.speed_of(i as u16) > 0.0;
        if looped {
            slot.flags |= anim_flags::LOOPED;
        }
    }
    for (event, block, c) in script.commands() {
        let flags = &mut info[usize::from(c.anim) + 1].flags;
        if c.weapon_time_scale {
            *flags |= anim_flags::WEAPON_TIME_SCALE;
        }
        if c.duration_ms.is_none() {
            *flags |= anim_flags::OWN_LENGTH;
        }
        *flags |= match (event, block) {
            (true, "fireweapon") => anim_flags::FIRE,
            (true, "death") => anim_flags::DEATH,
            (true, "knife_melee" | "knife_melee_charge") => anim_flags::KNIFE,
            (true, "scriptevent") => anim_flags::SCRIPT_EVENT,
            (true, "reload") => anim_flags::RELOAD,
            (true, "dropweapon" | "raiseweapon" | "firstraiseweapon") => anim_flags::CUTS_RELOAD,
            (
                true,
                "stand_to_crouch" | "crouch_to_stand" | "crouch_to_prone" | "prone_to_crouch"
                | "prone_to_stand" | "prone_to_sprint",
            ) => anim_flags::STANCE,
            (false, "walk") => anim_flags::WALK,
            (false, "run") => anim_flags::RUN,
            _ => 0,
        };
    }
    Some(info)
}

/// The script as `third`'s table: conditions are the script's own words.
pub(super) struct ThirdTable<'s, 'f> {
    pub script: &'s T6PlayerAnims,
    pub info: &'s [AnimInfo],
    words: PhantomData<&'f str>,
}

impl<'s> ThirdTable<'s, '_> {
    pub fn new(script: &'s T6PlayerAnims, info: &'s [AnimInfo]) -> Self {
        Self {
            script,
            info,
            words: PhantomData,
        }
    }

    /// One script command as `third` plays it: its part, its anim, its `duration` or the
    /// anim's own length.
    pub fn command(&self, c: &T6AnimCommand) -> Command {
        let anim = c.anim + 1;
        let part = match c.part {
            T6AnimPart::Legs => Part::Legs,
            T6AnimPart::Torso => Part::Torso,
            T6AnimPart::Both => Part::Both,
        };
        let duration = c.duration_ms.unwrap_or_else(|| self.info(anim).len_ms);
        Command::one(part, anim, duration)
    }

    fn pick(
        &self,
        commands: &[T6AnimCommand],
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<Command> {
        if commands.is_empty() {
            return None;
        }
        let i = pick(commands.len()) % commands.len();
        Some(self.command(&commands[i]))
    }
}

impl<'f> AnimTable for ThirdTable<'_, 'f> {
    type Conds = T6AnimFacts<'f>;

    fn event_command(
        &self,
        event: AnimEvent,
        conds: &Self::Conds,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<Command> {
        // Row 17: the script has no riot shield block for standing up or crouching, so its
        // default would play the rifle's stand2crouch / crouch2stand, whose left arm swings
        // the shield up over his head. Real BO2 footage shows a shield carrier crouching and
        // standing in place with the shield planted: the stance only blends, as when moving.
        if matches!(event, AnimEvent::StandToCrouch | AnimEvent::CrouchToStand) && conds.anim_type == "riotshield" {
            return None;
        }
        self.pick(self.script.pick_event(event.script_name(), conds)?, pick)
    }

    fn move_command(
        &self,
        mt: MoveType,
        conds: &Self::Conds,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<Command> {
        self.pick(self.script.pick_move(mt.script_name(), conds)?, pick)
    }

    fn info(&self, anim: u16) -> AnimInfo {
        self.info
            .get(usize::from(third::anim_index(anim)))
            .copied()
            // An anim the script does not name: no flags, no length.
            .unwrap_or(AnimInfo {
                len_ms: 0,
                flags: 0,
                moves: false,
            })
    }
}

/// A third-person anim value as sim packs it (its restart bit as our toggle).
pub(super) fn packed(value: u16) -> i32 {
    let id = i32::from(third::anim_index(value));
    if id == 0 {
        return 0;
    }
    (id & T6_ANIM_INDEX_MASK) | if value & RESTART_BIT != 0 { T6_ANIM_TOGGLE } else { 0 }
}

/// The call (with BO2's continue / force flags) for each event sim still guesses from the
/// MW2 weapon states and stance changes, until the gun brain sends its own. Going prone
/// fires no event in BO2 (the legs' prone moves show it).
pub(super) fn guessed_call(event: &str) -> Option<AnimCall> {
    use AnimEvent as E;
    Some(match event {
        "fireweapon" => AnimCall::FIRE,
        "dropweapon" => AnimCall::DROP,
        "raiseweapon" => AnimCall::raise(false),
        "firstraiseweapon" => AnimCall::raise(true),
        "reload" => AnimCall::RELOAD,
        "meleeattack" => AnimCall::melee(E::MeleeAttack),
        "prime_grenade" => AnimCall::prime(false),
        "dtp_takeoff" => AnimCall::DTP_TAKEOFF,
        "dtp_land" => AnimCall::DTP_LAND,
        "run_to_idle" => AnimCall::RUN_TO_IDLE,
        "jump" => AnimCall::new(E::Jump, false, true),
        "land" => AnimCall::new(E::Land, false, true),
        "stand_to_crouch" => AnimCall::new(E::StandToCrouch, false, false),
        "crouch_to_stand" => AnimCall::new(E::CrouchToStand, false, false),
        "prone_to_crouch" => AnimCall::new(E::ProneToCrouch, false, true),
        "prone_to_stand" => AnimCall::new(E::ProneToStand, false, true),
        "prone_to_sprint" => AnimCall::new(E::ProneToSprint, false, true),
        _ => return None,
    })
}

/// The script's word and grenade flag for an anim script event the T6 brain fired, in the
/// words the guessed events use (so the same start is not played twice).
pub(super) fn script_word(script: AnimScript) -> (&'static str, bool) {
    use AnimScript as A;
    match script {
        A::DropWeapon => ("dropweapon", false),
        A::RaiseWeapon => ("raiseweapon", false),
        A::FirstRaiseWeapon => ("firstraiseweapon", false),
        A::PrimeGrenade | A::PrimeGear => ("prime_grenade", true),
        A::Reload => ("reload", false),
        A::FireWeapon | A::RiotshieldStart => ("fireweapon", false),
        A::OffHandStart => ("fireweapon", true),
        A::DtpTakeoff => ("dtp_takeoff", false),
        A::DtpLand => ("dtp_land", false),
        A::RunToIdle => ("run_to_idle", false),
        A::MeleeAttack => ("meleeattack", false),
        A::KnifeMelee => ("knife_melee", false),
        A::KnifeMeleeCharge => ("knife_melee_charge", false),
        A::MeleeLeft => ("meleeleft", false),
    }
}

/// The guessed words read from the gun's state. With the T6 brain on, the brain sends every one
/// of these itself, at BO2's own moment; the guesses read T6 states through iw4's names, where
/// OFFHAND_START (T6: the cook, key still held) is iw4's throw and OFFHAND_HOLD (T6: after the
/// key comes up) is iw4's cook, so a guessed grenade throw would play at the pin pull.
const GUN_WORDS: [&str; 7] =
    ["dropweapon", "raiseweapon", "reload", "meleeattack", "prime_grenade", "fireweapon", "firstraiseweapon"];

/// This frame's calls: the guessed events (from state changes), except the gun's own once the
/// T6 brain is on (it fires those itself), then the brain's with BO2's flags for each call
/// site. No brain = all guessed.
pub(super) fn third_calls(
    guessed: &[(&'static str, bool)],
    fed: Option<&[AnimScript]>,
) -> Vec<(&'static str, bool, AnimCall)> {
    let brain = fed.is_some();
    let fed: Vec<(&'static str, bool, AnimCall)> = fed
        .unwrap_or(&[])
        .iter()
        .map(|&s| {
            let (word, grenade) = script_word(s);
            (word, grenade, AnimCall::from_script(s))
        })
        .collect();
    let mut calls: Vec<_> = guessed
        .iter()
        .filter(|&&(w, _)| !(brain && GUN_WORDS.contains(&w)))
        .filter(|&&(w, g)| !fed.iter().any(|&(fw, fg, _)| same_kind(fw, w) && fg == g))
        .filter_map(|&(w, g)| guessed_call(w).map(|call| (w, g, call)))
        .collect();
    calls.extend(fed);
    calls
}

/// The guessed melee is one word for every swing; the brain's knife, lunge and left-hand swings
/// stand for it too.
fn same_kind(fed: &str, guessed: &str) -> bool {
    const MELEE: [&str; 4] = ["meleeattack", "knife_melee", "knife_melee_charge", "meleeleft"];
    fed == guessed || (MELEE.contains(&fed) && MELEE.contains(&guessed))
}

/// The script's word for BO2's move status (0 stationary, 1 walk, 2 run).
pub(super) fn movestatus_word(status: u8) -> &'static str {
    match status {
        0 => "stationary",
        1 => "walk",
        _ => "run",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brains_events_stand_for_the_guessed_ones_with_the_same_word() {
        let guessed = [("stand_to_crouch", false), ("reload", false), ("fireweapon", false)];
        let calls = third_calls(&guessed, Some(&[AnimScript::Reload, AnimScript::OffHandStart]));
        let words: Vec<_> = calls.iter().map(|c| (c.0, c.1)).collect();
        // The guessed gun words go (the brain sends its own); the stance change stays.
        assert_eq!(words, [("stand_to_crouch", false), ("reload", false), ("fireweapon", true)]);
        assert_eq!(calls[1].2, AnimCall::RELOAD);
        assert_eq!(calls[2].2, AnimCall::FIRE);
        // No brain: every guessed word through the guessed calls.
        assert_eq!(third_calls(&guessed, None).len(), 3);
    }

    #[test]
    fn a_cooked_grenade_plays_one_prime_and_one_throw_on_the_key_up() {
        // T6's cook (OFFHAND_START) reads as iw4's throw and its release (OFFHAND_HOLD) as iw4's
        // cook: those guesses, with the brain silent on the frame, play nothing.
        assert!(third_calls(&[("fireweapon", true)], Some(&[])).is_empty());
        assert!(third_calls(&[("prime_grenade", true)], Some(&[])).is_empty());
        // The brain's own: the prime at the pin pull, the throw when the key comes up.
        let pull = third_calls(&[("prime_grenade", true)], Some(&[AnimScript::PrimeGrenade]));
        assert_eq!(pull.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(), [("prime_grenade", true)]);
        let throw = third_calls(&[("fireweapon", true)], Some(&[AnimScript::OffHandStart]));
        assert_eq!(throw.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(), [("fireweapon", true)]);
    }

    #[test]
    fn a_stop_from_a_run_plays_its_block() {
        let calls = third_calls(&[("run_to_idle", false)], None);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, AnimCall::RUN_TO_IDLE);
    }

    #[test]
    fn a_sprint_up_from_prone_plays_its_block() {
        let calls = third_calls(&[("prone_to_sprint", false)], None);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, AnimCall::new(AnimEvent::ProneToSprint, false, true));
    }

    #[test]
    fn the_brains_knife_lunge_and_left_swings_keep_their_own_blocks() {
        let guessed = [("meleeattack", false)];
        for (script, word) in [
            (AnimScript::KnifeMelee, "knife_melee"),
            (AnimScript::KnifeMeleeCharge, "knife_melee_charge"),
            (AnimScript::MeleeLeft, "meleeleft"),
            (AnimScript::MeleeAttack, "meleeattack"),
        ] {
            let calls = third_calls(&guessed, Some(&[script]));
            let words: Vec<_> = calls.iter().map(|c| c.0).collect();
            // One swing: the brain's block, the guessed bash dropped.
            assert_eq!(words, [word]);
        }
    }

    fn script() -> T6PlayerAnims {
        let text = "ANIMATIONS\nSTATE COMBAT\n{\n\tidle\n\t{\n\t\tdefault\n\t\t{\n\t\t\tboth pb_idle\n\t\t}\n\t}\n}\nEVENTS\n\
            fireweapon\n{\n\tdefault\n\t{\n\t\ttorso pt_fire\n\t}\n}\n\
            reload\n{\n\tdefault\n\t{\n\t\ttorso pt_reload duration 2000\n\t}\n}\n\
            dropweapon\n{\n\tdefault\n\t{\n\t\ttorso pt_drop weaponTimeScale\n\t}\n}\n";
        T6PlayerAnims::parse(text, "default\n").expect("parses")
    }

    #[test]
    fn values_are_index_plus_one_and_flags_follow_the_blocks() {
        let s = script();
        let info = anim_infos(&s, |_| Some((400, false))).expect("fits");
        let table = ThirdTable::new(&s, &info);
        let id = |n: &str| s.anims.iter().position(|a| a == n).unwrap() as u16 + 1;
        assert_eq!(table.info(id("pt_fire")).flags & anim_flags::FIRE, anim_flags::FIRE);
        assert_ne!(table.info(id("pt_reload")).flags & anim_flags::RELOAD, 0);
        assert_eq!(table.info(id("pt_reload")).flags & anim_flags::OWN_LENGTH, 0);
        let drop = table.info(id("pt_drop")).flags;
        assert_ne!(drop & anim_flags::CUTS_RELOAD, 0);
        assert_ne!(drop & anim_flags::WEAPON_TIME_SCALE, 0);
        let facts = T6AnimFacts::default();
        let cmd = table
            .event_command(AnimEvent::Reload, &facts, &mut |_| 0)
            .expect("reload block");
        assert_eq!(cmd, Command::one(Part::Torso, id("pt_reload"), 2000));
        // Own length when the block gives no duration.
        let fire = table
            .event_command(AnimEvent::FireWeapon, &facts, &mut |_| 0)
            .expect("fire block");
        assert_eq!(fire.duration_ms, 400);
    }

    #[test]
    fn a_shield_carrier_crouches_and_stands_without_the_rifle_swing() {
        let text = "ANIMATIONS\nSTATE COMBAT\n{\n\tidle\n\t{\n\t\tdefault\n\t\t{\n\t\t\tboth pb_idle\n\t\t}\n\t}\n}\nEVENTS\n\
            stand_to_crouch\n{\n\tdefault\n\t{\n\t\tboth pb_s2c\n\t}\n}\n\
            crouch_to_stand\n{\n\tdefault\n\t{\n\t\tboth pb_c2s\n\t}\n}\n";
        let s = T6PlayerAnims::parse(text, "default\n").expect("parses");
        let info = anim_infos(&s, |_| Some((400, false))).expect("fits");
        let table = ThirdTable::new(&s, &info);
        let rifle = T6AnimFacts::default();
        let shield = T6AnimFacts { anim_type: "riotshield", ..T6AnimFacts::default() };
        for event in [AnimEvent::StandToCrouch, AnimEvent::CrouchToStand] {
            assert!(table.event_command(event, &rifle, &mut |_| 0).is_some());
            assert_eq!(table.event_command(event, &shield, &mut |_| 0), None);
        }
    }

    #[test]
    fn packing_maps_the_restart_bit_to_our_toggle() {
        assert_eq!(packed(0), 0);
        assert_eq!(packed(RESTART_BIT), 0);
        assert_eq!(packed(5), 5);
        assert_eq!(packed(5 | RESTART_BIT), 5 | T6_ANIM_TOGGLE);
    }

    #[test]
    fn a_reload_is_cut_by_a_swap_but_not_by_a_shot() {
        let s = script();
        let info = anim_infos(&s, |_| Some((400, false))).expect("fits");
        let table = ThirdTable::new(&s, &info);
        let facts = T6AnimFacts::default();
        let mut body = third::Body::default();
        let mut seed = 1;
        let gun = third::GunTimes {
            weapon_time: 600,
            weapon_delay: 0,
        };
        third::script_event(&mut body, AnimCall::RELOAD, &facts, &table, gun, &mut seed, 0);
        let reload = body.torso_anim;
        body.tick(100);
        // Not forced and the torso is busy: a raise would not start, but a drop cuts a reload.
        third::script_event(&mut body, AnimCall::DROP, &facts, &table, gun, &mut seed, 0);
        assert_ne!(third::anim_index(body.torso_anim), third::anim_index(reload));
    }
}
