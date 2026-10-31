//! BO2's perk numbers: the one table every `ps.perks` reader and writer uses.
//!
//! BO2's `BG_HasPerk` tests bit `perk % 32` of `perks[perk / 32]`
//! ([`crate::AvatarState::has_perk_bit`], [`crate::AvatarState::set_perk`]), so a perk
//! constant here is its `perksEnum` number, never a bit mask. `PERK_UNKNOWN` (63, past
//! `PERK_COUNT`) is BO2's "no perk": `has_perk_bit` is false for it.

pub const PERK_ADDITIONALPRIMARYWEAPON: u32 = 0;
pub const PERK_ARMORPIERCING: u32 = 1;
pub const PERK_ARMORVEST: u32 = 2;
pub const PERK_ACCURACY: u32 = 3;
pub const PERK_BULLETDMG: u32 = 4;
pub const PERK_REDUCEBULLETFLINCH: u32 = 5;
pub const PERK_EXTRABP: u32 = 6;
pub const PERK_DEADSHOT: u32 = 7;
pub const PERK_DELAYEXPLOSIVE: u32 = 8;
pub const PERK_DETECTEXPLOSIVE: u32 = 9;
pub const PERK_DISARMEXPLOSIVE: u32 = 10;
pub const PERK_EARNMOREMOMENTUM: u32 = 11;
pub const PERK_EXPLOSIVEDMG: u32 = 12;
pub const PERK_EXTRAAMMO: u32 = 13;
/// specialty_fallheight.
pub const PERK_FALLHEIGHT: u32 = 14;
/// specialty_fastads (Quickdraw).
pub const PERK_FASTADS: u32 = 15;
pub const PERK_FASTEQUIPMENTUSE: u32 = 16;
/// specialty_fastladderclimb.
pub const PERK_FASTLADDERCLIMB: u32 = 17;
/// specialty_fastmantle.
pub const PERK_FASTMANTLE: u32 = 18;
pub const PERK_FASTMELEE: u32 = 19;
pub const PERK_FASTRELOAD: u32 = 20;
pub const PERK_FASTTOSS: u32 = 21;
pub const PERK_FASTSWITCH: u32 = 22;
pub const PERK_FINALSTAND: u32 = 23;
pub const PERK_FIREPROOF: u32 = 24;
/// specialty_flakjacket, Zombies' PhD Flopper.
pub const PERK_FLAKJACKET: u32 = 25;
pub const PERK_FLASHPROTECTION: u32 = 26;
pub const PERK_JAMRADAR: u32 = 27;
pub const PERK_GRENADEDEATH: u32 = 28;
pub const PERK_HEALTHREGEN: u32 = 29;
pub const PERK_EXTRABREATH: u32 = 30;
pub const PERK_IMMUNECOUNTERUAV: u32 = 31;
pub const PERK_IMMUNEEMP: u32 = 32;
pub const PERK_IMMUNEMMS: u32 = 33;
pub const PERK_IMMUNENVTHERMAL: u32 = 34;
pub const PERK_IMMUNERANGEFINDER: u32 = 35;
pub const PERK_KILLSTREAK: u32 = 36;
/// specialty_longersprint (Stamin-Up, multiplayer's Extreme Conditioning).
pub const PERK_LONGERSPRINT: u32 = 37;
pub const PERK_LOUDENEMIES: u32 = 38;
pub const PERK_MARKSMAN: u32 = 39;
/// specialty_movefaster (multiplayer's Lightweight speed, the Turned zombies' perk).
pub const PERK_MOVEFASTER: u32 = 40;
pub const PERK_NOMOTIONSENSOR: u32 = 41;
pub const PERK_NONAME: u32 = 42;
pub const PERK_NOTTARGETEDBYAIRSUPPORT: u32 = 43;
pub const PERK_NOKILLSTREAKRETICLE: u32 = 44;
pub const PERK_NOTTARGETEDBYSENTRY: u32 = 45;
pub const PERK_TOSSBACK: u32 = 46;
/// specialty_pistoldeath (Second Chance).
pub const PERK_PISTOLDEATH: u32 = 47;
pub const PERK_PROXIMITYPROTECTION: u32 = 48;
pub const PERK_QUICKREVIVE: u32 = 49;
/// specialty_quieter (Ninja).
pub const PERK_QUIETMOVE: u32 = 50;
pub const PERK_RECON: u32 = 51;
pub const PERK_RATEOFFIRE: u32 = 52;
/// specialty_scavenger.
pub const PERK_SCAVENGER: u32 = 53;
pub const PERK_SHOWENEMYEQUIPMENT: u32 = 54;
pub const PERK_STUNPROTECTION: u32 = 55;
pub const PERK_REDUCESHELLSHOCK: u32 = 56;
pub const PERK_SPRINTRECOVERY: u32 = 57;
pub const PERK_SHOWONRADAR: u32 = 58;
/// specialty_stalker: aiming does not slow the walk.
pub const PERK_STALKER: u32 = 59;
pub const PERK_TWOGRENADES: u32 = 60;
pub const PERK_TWOPRIMARIES: u32 = 61;
/// specialty_unlimitedsprint, which Marathon is in BO2.
pub const PERK_UNLIMITEDSPRINT: u32 = 62;
/// BO2's `PERK_UNKNOWN` (equal to `PERK_COUNT`): no perk.
pub const PERK_UNKNOWN: u32 = 63;

/// Every perk number in `perksEnum` order: 0 up to `PERK_COUNT`, so each constant above in turn.
pub const PERKS_IN_ENUM_ORDER: [u32; 63] = {
    let mut all = [0; 63];
    let mut n = 0;
    while n < all.len() {
        all[n] = n as u32;
        n += 1;
    }
    all
};

/// The script perk names (`setperk` / `hasperk`) and their BO2 perk numbers. A name with no
/// entry (BO2's scripts never use it, or its place in `perksEnum` is not settled) has no perk
/// bit.
pub const PERK_NAMES: &[(&str, u32)] = &[
    ("specialty_armorpiercing", PERK_ARMORPIERCING),
    ("specialty_bulletaccuracy", PERK_ACCURACY),
    ("specialty_bulletflinch", PERK_REDUCEBULLETFLINCH),
    ("specialty_bulletpenetration", PERK_EXTRABP),
    ("specialty_fallheight", PERK_FALLHEIGHT),
    ("specialty_fastads", PERK_FASTADS),
    ("specialty_fastequipmentuse", PERK_FASTEQUIPMENTUSE),
    ("specialty_fastladderclimb", PERK_FASTLADDERCLIMB),
    ("specialty_fastmantle", PERK_FASTMANTLE),
    ("specialty_fastmeleerecovery", PERK_FASTMELEE),
    ("specialty_fastreload", PERK_FASTRELOAD),
    ("specialty_fasttoss", PERK_FASTTOSS),
    ("specialty_fastweaponswitch", PERK_FASTSWITCH),
    ("specialty_flakjacket", PERK_FLAKJACKET),
    ("specialty_longersprint", PERK_LONGERSPRINT),
    ("specialty_movefaster", PERK_MOVEFASTER),
    ("specialty_pistoldeath", PERK_PISTOLDEATH),
    ("specialty_quieter", PERK_QUIETMOVE),
    ("specialty_rof", PERK_RATEOFFIRE),
    ("specialty_scavenger", PERK_SCAVENGER),
    ("specialty_sprintrecovery", PERK_SPRINTRECOVERY),
    ("specialty_stalker", PERK_STALKER),
    ("specialty_unlimitedsprint", PERK_UNLIMITEDSPRINT),
];

/// The BO2 perk number a script perk name stands for.
#[must_use]
pub fn by_name(name: &str) -> Option<u32> {
    PERK_NAMES.iter().find(|(perk, _)| *perk == name).map(|&(_, number)| number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_perk_is_its_perks_enum_number() {
        // Each named constant against the number perksEnum gives it, in that order.
        let numbered: [(u32, u32); 63] = [
            (PERK_ADDITIONALPRIMARYWEAPON, 0), (PERK_ARMORPIERCING, 1), (PERK_ARMORVEST, 2),
            (PERK_ACCURACY, 3), (PERK_BULLETDMG, 4), (PERK_REDUCEBULLETFLINCH, 5),
            (PERK_EXTRABP, 6), (PERK_DEADSHOT, 7), (PERK_DELAYEXPLOSIVE, 8),
            (PERK_DETECTEXPLOSIVE, 9), (PERK_DISARMEXPLOSIVE, 10), (PERK_EARNMOREMOMENTUM, 11),
            (PERK_EXPLOSIVEDMG, 12), (PERK_EXTRAAMMO, 13), (PERK_FALLHEIGHT, 14),
            (PERK_FASTADS, 15), (PERK_FASTEQUIPMENTUSE, 16), (PERK_FASTLADDERCLIMB, 17),
            (PERK_FASTMANTLE, 18), (PERK_FASTMELEE, 19), (PERK_FASTRELOAD, 20), (PERK_FASTTOSS, 21),
            (PERK_FASTSWITCH, 22), (PERK_FINALSTAND, 23), (PERK_FIREPROOF, 24),
            (PERK_FLAKJACKET, 25), (PERK_FLASHPROTECTION, 26), (PERK_JAMRADAR, 27),
            (PERK_GRENADEDEATH, 28), (PERK_HEALTHREGEN, 29), (PERK_EXTRABREATH, 30),
            (PERK_IMMUNECOUNTERUAV, 31), (PERK_IMMUNEEMP, 32), (PERK_IMMUNEMMS, 33),
            (PERK_IMMUNENVTHERMAL, 34), (PERK_IMMUNERANGEFINDER, 35), (PERK_KILLSTREAK, 36),
            (PERK_LONGERSPRINT, 37), (PERK_LOUDENEMIES, 38), (PERK_MARKSMAN, 39),
            (PERK_MOVEFASTER, 40), (PERK_NOMOTIONSENSOR, 41), (PERK_NONAME, 42),
            (PERK_NOTTARGETEDBYAIRSUPPORT, 43), (PERK_NOKILLSTREAKRETICLE, 44),
            (PERK_NOTTARGETEDBYSENTRY, 45), (PERK_TOSSBACK, 46), (PERK_PISTOLDEATH, 47),
            (PERK_PROXIMITYPROTECTION, 48), (PERK_QUICKREVIVE, 49), (PERK_QUIETMOVE, 50),
            (PERK_RECON, 51), (PERK_RATEOFFIRE, 52), (PERK_SCAVENGER, 53),
            (PERK_SHOWENEMYEQUIPMENT, 54), (PERK_STUNPROTECTION, 55), (PERK_REDUCESHELLSHOCK, 56),
            (PERK_SPRINTRECOVERY, 57), (PERK_SHOWONRADAR, 58), (PERK_STALKER, 59),
            (PERK_TWOGRENADES, 60), (PERK_TWOPRIMARIES, 61), (PERK_UNLIMITEDSPRINT, 62),
        ];
        for (at, &(perk, number)) in numbered.iter().enumerate() {
            assert_eq!(perk, number);
            assert_eq!(PERKS_IN_ENUM_ORDER[at], perk);
        }
        assert_eq!(PERK_UNKNOWN, crate::PERK_COUNT);
        // A few against BO2's own table (types.h perksEnum).
        assert_eq!(PERK_FASTADS, 15);
        assert_eq!(PERK_FASTMANTLE, 18);
        assert_eq!(PERK_LONGERSPRINT, 37);
        assert_eq!(PERK_MOVEFASTER, 40);
        assert_eq!(PERK_STALKER, 59);
        assert_eq!(PERK_UNLIMITEDSPRINT, 62);
    }

    #[test]
    fn perks_land_on_bo2s_words_and_bits() {
        let mut ps = crate::AvatarState::default();
        ps.set_perk(PERK_LONGERSPRINT, true);
        assert_eq!(ps.perks[1], 1 << 5);
        assert!(ps.has_perk_bit(PERK_LONGERSPRINT));
        assert!(!ps.has_perk_bit(PERK_FASTADS));
        ps.set_perk(PERK_FASTADS, true);
        assert_eq!(ps.perks[0], 1 << 15);
        ps.set_perk(PERK_UNLIMITEDSPRINT, true);
        assert_eq!(ps.perks[1], (1 << 5) | (1 << 30));
        ps.set_perk(PERK_FASTADS, false);
        assert_eq!(ps.perks[0], 0);
    }

    #[test]
    fn no_perk_is_never_held() {
        let mut ps = crate::AvatarState::default();
        ps.perks = [u32::MAX; 2];
        assert!(!ps.has_perk_bit(PERK_UNKNOWN));
        ps.set_perk(PERK_UNKNOWN, true);
        assert_eq!(ps.perks, [u32::MAX; 2]);
    }

    #[test]
    fn script_names_find_their_numbers_and_each_appears_once() {
        assert_eq!(by_name("specialty_movefaster"), Some(PERK_MOVEFASTER));
        assert_eq!(by_name("specialty_unlimitedsprint"), Some(62));
        assert_eq!(by_name("specialty_no_such_perk"), None);
        for (at, (name, perk)) in PERK_NAMES.iter().enumerate() {
            assert!(*perk < PERK_UNKNOWN, "{name}");
            assert!(PERK_NAMES[at + 1..].iter().all(|(other, _)| other != name), "{name}");
        }
    }
}
