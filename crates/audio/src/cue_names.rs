//! Names of the footstep, landing and gear sounds, built once per process.
//!
//! Every name is generated up front from the surface list and interned, so the
//! rest of the game can pass `&'static str` around and look names up by alias.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use movement_t6::bridge::{SURFACE_KIND_LABELS, material_kind_number};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FootfallPace {
    Sprint,
    Run,
    Walking,
    Prone,
    /// Black Ops II's crouched steps and a hand on a ladder rung.
    CrouchRun,
    CrouchWalk,
    LadderHand,
}

impl FootfallPace {
    pub const ALL_KINDS: [Self; 7] = [
        Self::Sprint,
        Self::Run,
        Self::Walking,
        Self::Prone,
        Self::CrouchRun,
        Self::CrouchWalk,
        Self::LadderHand,
    ];

    /// The gait whose sounds a generic bank has for this one: crouched and
    /// ladder steps exist only in Black Ops II, so a generic bank plays a run
    /// or a walk instead.
    pub fn fallback(self) -> Self {
        match self {
            Self::CrouchRun | Self::LadderHand => Self::Run,
            Self::CrouchWalk => Self::Walking,
            unmatched => unmatched,
        }
    }
}

/// What a generated name turns into: its quieter form, and its form as heard
/// from another player.
#[derive(Clone, Copy)]
struct Forms {
    softer: Option<&'static str>,
    heard: Option<&'static str>,
}

struct Bank {
    /// Alias and the alias to play when it is missing, keyed by
    /// (gait, or None for a landing; our own; quieter; surface slot).
    picks: HashMap<(Option<FootfallPace>, bool, bool, usize), (&'static str, &'static str)>,
    /// Every name worth trying for an alias, best first.
    tries: HashMap<&'static str, &'static [&'static str]>,
    forms: HashMap<&'static str, Forms>,
    /// The names a generic bank may be asked for: Black Ops II's own steps left out.
    generic_names: Vec<&'static str>,
    /// Black Ops II's own steps: what the bank makes on top of `generic_names`.
    exclusive_names: Vec<&'static str>,
}

/// `step_x` becomes `qstep_x` and `Land_x` becomes `qLand_x`: the quieter set.
fn quiet_name(cue_id: &str) -> Option<String> {
    let (head, rest) = match cue_id.strip_prefix("step_") {
        Some(rest) => ("qstep_", rest),
        None => ("qLand_", cue_id.strip_prefix("Land_")?),
    };
    Some(format!("{head}{rest}"))
}

/// `step_run_plr_x` becomes `step_run_x`: the same sound as heard from another player.
fn world_name(cue_id: &str) -> Option<String> {
    let (before, after) = cue_id.split_once("_plr_")?;
    Some(format!("{before}_{after}"))
}

/// The names to try for `alias`, then `fallback`: each as given, in its
/// world form, and in its quiet forms; no repeats.
fn try_order(cue_id: &str, backup_name: &str) -> Vec<String> {
    let expand = |seed: &str| {
        let softer = quiet_name(seed);
        let softer_heard = softer.as_deref().and_then(world_name);
        [Some(seed.to_owned()), world_name(seed), softer, softer_heard]
    };
    let mut order = Vec::with_capacity(8);
    let candidates = expand(cue_id).into_iter().chain(expand(backup_name)).flatten();
    for name in candidates.filter(|n| !n.is_empty()) {
        if !order.contains(&name) {
            order.push(name);
        }
    }
    order
}

/// Puts a name into its quiet form when asked and it has one.
fn quietened(quiet: bool, name: String) -> String {
    match quiet.then(|| quiet_name(&name)).flatten() {
        Some(softer) => softer,
        None => name,
    }
}

#[derive(Default)]
struct Registry {
    interned: HashSet<&'static str>,
    generic: HashSet<&'static str>,
    exclusive: HashSet<&'static str>,
    tries: HashMap<&'static str, &'static [&'static str]>,
    forms: HashMap<&'static str, Forms>,
}

impl Registry {
    /// One leaked copy per distinct name.
    fn intern(&mut self, words: String) -> &'static str {
        if let Some(&known) = self.interned.get(words.as_str()) {
            return known;
        }
        let words: &'static str = Box::leak(words.into_boxed_str());
        self.interned.insert(words);
        words
    }

    /// Records a pick and every name it can resolve to.
    /// Interns one candidate name, files it as generic or Black Ops II only,
    /// and remembers its quiet and world forms.
    fn note_name(&mut self, cue_name: String, exclusive: bool) -> &'static str {
        let cue_name = self.intern(cue_name);
        let set = if exclusive { &mut self.exclusive } else { &mut self.generic };
        set.insert(cue_name);
        let softer = quiet_name(cue_name).map(|s| self.intern(s));
        let heard = world_name(cue_name).map(|s| self.intern(s));
        self.forms.insert(cue_name, Forms { softer, heard });
        cue_name
    }

    /// Records an alias with its fallback and every name they can resolve to.
    fn register(&mut self, alias: String, spare_name: String, exclusive: bool) -> (&'static str, &'static str) {
        let listed: Vec<&'static str> = try_order(&alias, &spare_name)
            .into_iter()
            .map(|name| self.note_name(name, exclusive))
            .collect();
        let pick = (self.intern(alias), self.intern(spare_name));
        self.tries.insert(pick.0, Box::leak(listed.into_boxed_slice()));
        pick
    }
}

fn build_bank() -> Bank {
    let mut reg = Registry::default();
    let mut picks = HashMap::new();
    let surfaces = || SURFACE_KIND_LABELS.iter().enumerate();

    // Every gait's steps, then (None) the landings; each for others and for us.
    for gait in FootfallPace::ALL_KINDS.into_iter().map(Some).chain([None]) {
        let exclusive = gait.is_some_and(|g| g.fallback() != g);
        for own in [false, true] {
            let stem = match gait {
                Some(g) => {
                    reg.generic.insert(gear_jingle_cue(g, own));
                    stride_prefix(g, own)
                }
                None => ["Land", "Land_plr"][usize::from(own)],
            };
            let cells = [false, true].into_iter().flat_map(|q| surfaces().map(move |s| (q, s)));
            for (quiet, (surface, terrain)) in cells {
                let alias = quietened(quiet, format!("{stem}_{terrain}"));
                let fallback = quietened(quiet, format!("{stem}_default"));
                picks.insert((gait, own, quiet, surface), reg.register(alias, fallback, exclusive));
            }
        }
    }
    reg.generic.extend([gear_alias(true), gear_alias(false)]);

    let mut generic_names: Vec<_> = reg.generic.iter().copied().collect();
    generic_names.sort_unstable();
    let exclusive_names = reg.exclusive.difference(&reg.generic).copied().collect();
    Bank { picks, tries: reg.tries, forms: reg.forms, generic_names, exclusive_names }
}

static CUE_REGISTRY: LazyLock<Bank> = LazyLock::new(build_bank);

/// Surface flags map to a surface index; anything out of range is "default".
fn surface_slot(surface_flags: u32) -> usize {
    Some(material_kind_number(surface_flags))
        .filter(|&slot| slot < SURFACE_KIND_LABELS.len())
        .unwrap_or(0)
}

fn bank_pick(key: (Option<FootfallPace>, bool, bool, usize)) -> (&'static str, &'static str) {
    CUE_REGISTRY.picks[&key]
}

/// The start of every footstep alias of a gait, for ourselves or for others.
pub fn stride_prefix(gait: FootfallPace, is_own_player: bool) -> &'static str {
    let [others, own] = match gait {
        FootfallPace::Sprint => ["step_sprint", "step_sprint_plr"],
        FootfallPace::Run => ["step_run", "step_run_plr"],
        FootfallPace::Walking => ["step_walk", "step_walk_plr"],
        FootfallPace::Prone => ["step_prone", "step_prone_plr"],
        FootfallPace::CrouchRun => ["step_crouchrun", "step_crouchrun_plr"],
        FootfallPace::CrouchWalk => ["step_crouchwalk", "step_crouchwalk_plr"],
        FootfallPace::LadderHand => ["step_ladderhand", "step_ladderhand_plr"],
    };
    if is_own_player { own } else { others }
}

/// The footstep alias and its fallback for a gait on a surface.
pub fn footfall_cue_names(
    gait: FootfallPace,
    surf_bits: u32,
    is_own_player: bool,
    hushed: bool,
) -> (&'static str, &'static str) {
    bank_pick((Some(gait), is_own_player, hushed, surface_slot(surf_bits)))
}

/// The landing alias and its fallback for a surface.
pub fn touchdown_cue_names(
    surf_mask: u32,
    is_own_player: bool,
    lower_vol: bool,
) -> (&'static str, &'static str) {
    bank_pick((None, is_own_player, lower_vol, surface_slot(surf_mask)))
}

/// The same sound as another player's footstep, for a first-person alias.
pub fn surface_cue_for(alias: &str) -> Option<&'static str> {
    CUE_REGISTRY.forms.get(alias).and_then(|f| f.heard)
}

/// The quieter set's name for an alias.
pub fn hushed_ground_cue(alias: &str) -> Option<&'static str> {
    CUE_REGISTRY.forms.get(alias).and_then(|f| f.softer)
}

/// Every name worth trying for a footstep or landing alias, best first;
/// empty for an alias the bank did not generate.
pub fn surface_cue_list(alias: &str, _backup_cue: &str) -> &'static [&'static str] {
    CUE_REGISTRY.tries.get(alias).copied().unwrap_or(&[])
}

/// The rattle of a soldier's gear for a gait. A generic bank has no crouched
/// or ladder gear, so those use the run's and the walk's.
pub fn gear_jingle_cue(gait: FootfallPace, is_own_player: bool) -> &'static str {
    let [others, own] = match gait.fallback() {
        FootfallPace::Sprint => ["gear_rattle_sprint", "gear_rattle_plr_sprint"],
        FootfallPace::Walking => ["gear_rattle_walk", "gear_rattle_plr_walk"],
        FootfallPace::Prone => ["gear_rattle_prone", "gear_rattle_plr_prone"],
        _ => ["gear_rattle_run", "gear_rattle_plr_run"],
    };
    if is_own_player { own } else { others }
}

/// The gear rattle for a mantle.
pub fn gear_alias(is_own_player: bool) -> &'static str {
    ["gear_rattle_mantle", "gear_rattle_plr_mantle"][usize::from(is_own_player)]
}

/// Splits off who hears the sound: `plr_` marks the local player, anything
/// else is another player.
fn hearer(rest: &str) -> (&'static str, &str) {
    match rest.strip_prefix("plr_") {
        Some(tail) => ("plr", tail),
        None => ("npc", rest),
    }
}

/// The Black Ops II name for one of our movement aliases, if it has one.
///
/// Its footstep tables list names per surface: steps are
/// `fly_step_<gait>_<plr|npc>_<surface>`, landings `fly_land_<plr|npc>_<surface>`
/// and gear `fly_gear_<gait>_<plr|npc>`. It keeps no separate quiet set, so a
/// quiet alias maps to the plain name.
fn black_ops_movement_name(cue_id: &str) -> Option<String> {
    let plain = ["qstep_", "qLand_"]
        .iter()
        .find(|quiet| cue_id.starts_with(**quiet))
        .map_or(cue_id, |_| &cue_id[1..]);

    if let Some(rest) = plain.strip_prefix("step_") {
        let (gait, rest) = rest.split_once('_')?;
        // Our one-word gait names for Black Ops II's two-word ones.
        let gait = match gait {
            "crouchrun" => "crouch_run",
            "crouchwalk" => "crouch_walk",
            "ladderhand" => "ladder_hand",
            no_match => no_match,
        };
        let (who, surface) = hearer(rest);
        return Some(format!("fly_step_{gait}_{who}_{surface}"));
    }

    if let Some(rest) = plain.strip_prefix("Land_") {
        let (who, surface) = hearer(rest);
        return Some(format!("fly_land_{who}_{surface}"));
    }

    let rest = cue_id.strip_prefix("gear_rattle_")?;
    // Another player's mantle uses Black Ops II's prone-and-mantle gear; it
    // has no fly_gear_mantle_npc.
    if rest == "mantle" {
        return Some("fly_prone_n_mantle_npc".to_owned());
    }
    let (who, gait) = hearer(rest);
    Some(format!("fly_gear_{gait}_{who}"))
}

static BLACK_OPS_NAMES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    CUE_REGISTRY.generic_names
        .iter()
        .chain(&CUE_REGISTRY.exclusive_names)
        .filter_map(|&cue_name| {
            let theirs = black_ops_movement_name(cue_name)?;
            Some((cue_name, &*Box::leak(theirs.into_boxed_str())))
        })
        .collect()
});

/// The Black Ops II name of a footstep, landing or gear sound.
pub fn t6_movement_alias(alias: &str) -> Option<&'static str> {
    BLACK_OPS_NAMES.get(alias).copied()
}

/// The movement names a generic (non-Black Ops II) bank can be asked for.
pub fn motion_preload_cues() -> &'static [&'static str] {
    &CUE_REGISTRY.generic_names
}

/// The shot alias to use for a view: the first-person one when we are the
/// player and it is set, else the shared one. Empty names count as unset.
pub fn pick_gunshot_cue<'a>(
    first_person: bool,
    fire_cue: Option<&'a str>,
    trigger_sound_name: Option<&'a str>,
) -> Option<&'a str> {
    let set = |name: Option<&'a str>| name.filter(|name| !name.is_empty());
    let fired_by_me = if first_person { set(trigger_sound_name) } else { None };
    fired_by_me.or_else(|| set(fire_cue))
}

/// Like `pick_gunshot_cue`, but the last round of a magazine prefers the
/// weapon's last-shot aliases.
pub fn pick_view_gunshot_cue<'a>(
    prior_shot: bool,
    pov: bool,
    fire_snd: Option<&'a str>,
    trigger_sound_name: Option<&'a str>,
    final_round_fired: Option<&'a str>,
    last_shot_cue: Option<&'a str>,
) -> Option<&'a str> {
    prior_shot
        .then(|| pick_gunshot_cue(pov, final_round_fired, last_shot_cue))
        .flatten()
        .or_else(|| pick_gunshot_cue(pov, fire_snd, trigger_sound_name))
}

#[cfg(test)]
mod black_ops_name_tests {
    use super::*;

    const ASPHALT: u32 = 22 << 20;
    const CONCRETE: u32 = 5 << 20;
    const METAL: u32 = 13 << 20;

    fn step_name(gait: FootfallPace, flags: u32, local: bool, quieter: bool) -> Option<&'static str> {
        let (alias, _) = footfall_cue_names(gait, flags, local, quieter);
        t6_movement_alias(alias)
    }

    #[test]
    fn steps_landings_and_gear_take_black_ops_names() {
        assert_eq!(
            step_name(FootfallPace::Run, ASPHALT, true, false),
            Some("fly_step_run_plr_asphalt")
        );
        assert_eq!(
            step_name(FootfallPace::Sprint, CONCRETE, false, true),
            Some("fly_step_sprint_npc_concrete")
        );

        let (land, fallback) = touchdown_cue_names(ASPHALT, true, false);
        assert_eq!(t6_movement_alias(land), Some("fly_land_plr_asphalt"));
        assert_eq!(t6_movement_alias(fallback), Some("fly_land_plr_default"));

        let sprint_gear = gear_jingle_cue(FootfallPace::Sprint, true);
        assert_eq!(t6_movement_alias(sprint_gear), Some("fly_gear_sprint_plr"));
        assert_eq!(t6_movement_alias(gear_alias(true)), Some("fly_gear_mantle_plr"));
    }

    #[test]
    fn crouched_and_ladder_steps_take_two_word_gait_names() {
        let (alias, fallback) = footfall_cue_names(FootfallPace::CrouchRun, METAL, true, false);
        assert_eq!(t6_movement_alias(alias), Some("fly_step_crouch_run_plr_metal"));
        assert_eq!(
            t6_movement_alias(fallback),
            Some("fly_step_crouch_run_plr_default")
        );
        assert_eq!(
            step_name(FootfallPace::CrouchWalk, CONCRETE, false, false),
            Some("fly_step_crouch_walk_npc_concrete")
        );
        assert_eq!(
            step_name(FootfallPace::LadderHand, METAL, true, true),
            Some("fly_step_ladder_hand_plr_metal")
        );
        assert_eq!(
            step_name(FootfallPace::LadderHand, METAL, false, false),
            Some("fly_step_ladder_hand_npc_metal")
        );
    }

    #[test]
    fn a_generic_bank_is_never_asked_for_black_ops_only_steps() {
        let names = motion_preload_cues();
        for exclusive in ["crouchrun", "crouchwalk", "ladderhand"] {
            assert!(names.iter().all(|name| !name.contains(exclusive)));
        }
        assert!(names.contains(&"step_run_plr_metal"));

        assert_eq!(FootfallPace::CrouchRun.fallback(), FootfallPace::Run);
        assert_eq!(FootfallPace::CrouchWalk.fallback(), FootfallPace::Walking);
        assert_eq!(FootfallPace::LadderHand.fallback(), FootfallPace::Run);
        assert_eq!(FootfallPace::Prone.fallback(), FootfallPace::Prone);
    }

    #[test]
    fn quiet_and_world_forms_rewrite_the_alias() {
        assert_eq!(quiet_name("step_run_metal").as_deref(), Some("qstep_run_metal"));
        assert_eq!(quiet_name("Land_plr_dirt").as_deref(), Some("qLand_plr_dirt"));
        assert_eq!(quiet_name("gear_rattle_run"), None);
        assert_eq!(world_name("step_run_plr_metal").as_deref(), Some("step_run_metal"));
        assert_eq!(world_name("step_run_metal"), None);
    }

    #[test]
    fn candidates_run_alias_first_then_fallback_without_repeats() {
        let order = try_order("step_run_plr_metal", "step_run_plr_default");
        assert_eq!(
            order,
            [
                "step_run_plr_metal",
                "step_run_metal",
                "qstep_run_plr_metal",
                "qstep_run_metal",
                "step_run_plr_default",
                "step_run_default",
                "qstep_run_plr_default",
                "qstep_run_default",
            ]
        );
        assert_eq!(try_order("gear_x", "gear_x"), ["gear_x"]);
    }

    #[test]
    fn the_bank_lists_what_it_generated_and_nothing_else() {
        let (alias, _) = footfall_cue_names(FootfallPace::Walking, METAL, true, false);
        assert_eq!(surface_cue_list(alias, "")[0], alias);
        assert!(surface_cue_list("nothing_like_it", "").is_empty());
        assert_eq!(surface_cue_for(alias), Some("step_walk_metal"));
        assert_eq!(hushed_ground_cue(alias), Some("qstep_walk_plr_metal"));
    }

    #[test]
    fn quiet_steps_use_the_quiet_names() {
        let (loud, _) = footfall_cue_names(FootfallPace::Run, METAL, false, false);
        let (soft, _) = footfall_cue_names(FootfallPace::Run, METAL, false, true);
        assert_eq!(quiet_name(loud).as_deref(), Some(soft));
    }

    #[test]
    fn gear_follows_the_gait_it_falls_back_to() {
        assert_eq!(gear_jingle_cue(FootfallPace::CrouchRun, true), "gear_rattle_plr_run");
        assert_eq!(gear_jingle_cue(FootfallPace::LadderHand, false), "gear_rattle_run");
        assert_eq!(gear_jingle_cue(FootfallPace::CrouchWalk, false), "gear_rattle_walk");
        assert_eq!(gear_jingle_cue(FootfallPace::Prone, true), "gear_rattle_plr_prone");
        assert_eq!(
            t6_movement_alias(gear_alias(false)),
            Some("fly_prone_n_mantle_npc")
        );
    }

    #[test]
    fn prefixes_mark_the_local_player() {
        let pairs = FootfallPace::ALL_KINDS.map(|g| (stride_prefix(g, false), stride_prefix(g, true)));
        for (others, own) in pairs {
            assert_eq!(own, format!("{others}_plr"));
        }
    }

    #[test]
    fn fire_aliases_prefer_the_players_own_when_set() {
        assert_eq!(pick_gunshot_cue(true, Some("a"), Some("b")), Some("b"));
        assert_eq!(pick_gunshot_cue(false, Some("a"), Some("b")), Some("a"));
        assert_eq!(pick_gunshot_cue(true, Some("a"), Some("")), Some("a"));
        assert_eq!(pick_gunshot_cue(true, Some(""), None), None);
    }

    #[test]
    fn the_last_round_prefers_the_last_shot_aliases() {
        let pick = |last, view| {
            pick_view_gunshot_cue(last, view, Some("f"), Some("fp"), Some("l"), Some("lp"))
        };
        assert_eq!(pick(false, true), Some("fp"));
        assert_eq!(pick(true, true), Some("lp"));
        assert_eq!(pick(true, false), Some("l"));
        let no_last = pick_view_gunshot_cue(true, true, Some("f"), None, None, None);
        assert_eq!(no_last, Some("f"));
    }
}
