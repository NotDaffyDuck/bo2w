//! Names of the footstep, landing and gear sounds, built once per process.
//!
//! Every name is generated up front from the surface list and leaked, so the
//! rest of the game can pass `&'static str` around and look names up by alias.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use movement_t6::bridge::{SURFACE_TYPE_NAMES, surface_type_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepGait {
    Sprint,
    Run,
    Walk,
    Prone,
    /// Black Ops II's crouched steps and a hand on a ladder rung.
    CrouchRun,
    CrouchWalk,
    LadderHand,
}

impl StepGait {
    pub const ALL: [Self; 7] = [
        Self::Sprint,
        Self::Run,
        Self::Walk,
        Self::Prone,
        Self::CrouchRun,
        Self::CrouchWalk,
        Self::LadderHand,
    ];

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|&gait| gait == self)
            .expect("every gait is listed in ALL")
    }

    /// The gait whose sounds a generic bank has for this one: crouched and
    /// ladder steps exist only in Black Ops II, so a generic bank plays a run
    /// or a walk instead.
    pub fn fallback(self) -> Self {
        match self {
            Self::CrouchRun | Self::LadderHand => Self::Run,
            Self::CrouchWalk => Self::Walk,
            other => other,
        }
    }
}

const GAITS: usize = StepGait::ALL.len();
const SURFACES: usize = SURFACE_TYPE_NAMES.len();

/// The alias to play for a step or landing and the one to use if it is missing.
#[derive(Clone, Copy)]
struct SurfacePick {
    alias: &'static str,
    fallback: &'static str,
}

struct SurfaceBank {
    /// Indexed by `footstep_slot`.
    footsteps: Vec<SurfacePick>,
    /// Indexed by `land_slot`.
    landings: Vec<SurfacePick>,
    /// Every name worth trying for an alias, best first.
    candidates: HashMap<&'static str, &'static [&'static str]>,
    quiet: HashMap<&'static str, &'static str>,
    world: HashMap<&'static str, &'static str>,
    /// All names the bank can produce, Black Ops II's own steps included.
    every_name: Vec<&'static str>,
    /// The names a generic bank may be asked for: Black Ops II's own steps left out.
    generic_names: Vec<&'static str>,
}

fn footstep_slot(gait: StepGait, local_player: bool, quieter: bool, surface: usize) -> usize {
    ((gait.index() * 2 + usize::from(local_player)) * 2 + usize::from(quieter)) * SURFACES
        + surface
}

fn land_slot(local_player: bool, quieter: bool, surface: usize) -> usize {
    (usize::from(local_player) * 2 + usize::from(quieter)) * SURFACES + surface
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// `step_x` becomes `qstep_x` and `Land_x` becomes `qLand_x`: the quieter set.
fn quiet_name(alias: &str) -> Option<String> {
    if let Some(rest) = alias.strip_prefix("step_") {
        Some(format!("qstep_{rest}"))
    } else {
        alias.strip_prefix("Land_").map(|rest| format!("qLand_{rest}"))
    }
}

/// `step_run_plr_x` becomes `step_run_x`: the same sound as heard from another player.
fn world_name(alias: &str) -> Option<String> {
    let at = alias.find("_plr_")?;
    Some(format!("{}_{}", &alias[..at], &alias[at + "_plr_".len()..]))
}

/// The names to try for `alias`, then `fallback`: each as given, in its
/// world form, and in its quiet forms; no repeats.
fn candidate_names(alias: &str, fallback: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(8);
    let mut add = |name: String| {
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    };
    for seed in [alias, fallback] {
        add(seed.to_owned());
        if let Some(world) = world_name(seed) {
            add(world);
        }
        if let Some(quiet) = quiet_name(seed) {
            let quiet_world = world_name(&quiet);
            add(quiet);
            if let Some(quiet_world) = quiet_world {
                add(quiet_world);
            }
        }
    }
    names
}

#[derive(Default)]
struct BankBuilder {
    generic: HashSet<&'static str>,
    black_ops_only: HashSet<&'static str>,
    candidates: HashMap<&'static str, &'static [&'static str]>,
    quiet: HashMap<&'static str, &'static str>,
    world: HashMap<&'static str, &'static str>,
}

impl BankBuilder {
    /// Registers a pick and every name it can resolve to.
    fn pick(&mut self, alias: String, fallback: String, black_ops_only: bool) -> SurfacePick {
        let names = candidate_names(&alias, &fallback);
        let alias = leak(alias);
        let fallback = leak(fallback);

        let mut interned: Vec<&'static str> = Vec::with_capacity(names.len());
        for name in names {
            let name = leak(name);
            let pool = if black_ops_only {
                &mut self.black_ops_only
            } else {
                &mut self.generic
            };
            pool.insert(name);
            if let Some(quiet) = quiet_name(name) {
                self.quiet.insert(name, leak(quiet));
            }
            if let Some(world) = world_name(name) {
                self.world.insert(name, leak(world));
            }
            interned.push(name);
        }
        self.candidates
            .insert(alias, Box::leak(interned.into_boxed_slice()));
        SurfacePick { alias, fallback }
    }
}

/// Applies the quiet naming to a pick's two names, when they have one.
fn maybe_quiet(quieter: bool, name: String) -> String {
    if quieter {
        quiet_name(&name).unwrap_or(name)
    } else {
        name
    }
}

fn build_bank() -> SurfaceBank {
    let mut builder = BankBuilder::default();
    let placeholder = builder.pick(
        "step_run_default".to_owned(),
        "step_run_default".to_owned(),
        false,
    );

    let mut footsteps = vec![placeholder; GAITS * 2 * 2 * SURFACES];
    for gait in StepGait::ALL {
        let black_ops_only = gait.fallback() != gait;
        for local_player in [false, true] {
            let prefix = step_prefix(gait, local_player);
            for quieter in [false, true] {
                for (surface, name) in SURFACE_TYPE_NAMES.iter().enumerate() {
                    let alias = maybe_quiet(quieter, format!("{prefix}_{name}"));
                    let fallback = maybe_quiet(quieter, format!("{prefix}_default"));
                    footsteps[footstep_slot(gait, local_player, quieter, surface)] =
                        builder.pick(alias, fallback, black_ops_only);
                }
            }
        }
    }

    let mut landings = vec![placeholder; 2 * 2 * SURFACES];
    for local_player in [false, true] {
        let prefix = if local_player { "Land_plr" } else { "Land" };
        for quieter in [false, true] {
            for (surface, name) in SURFACE_TYPE_NAMES.iter().enumerate() {
                let alias = maybe_quiet(quieter, format!("{prefix}_{name}"));
                let fallback = maybe_quiet(quieter, format!("{prefix}_default"));
                landings[land_slot(local_player, quieter, surface)] =
                    builder.pick(alias, fallback, false);
            }
        }
    }

    for gait in StepGait::ALL {
        for local_player in [false, true] {
            builder.generic.insert(gear_rattle_alias(gait, local_player));
        }
    }
    builder.generic.insert(gear_alias(true));
    builder.generic.insert(gear_alias(false));

    let mut generic_names: Vec<&'static str> = builder.generic.iter().copied().collect();
    generic_names.sort_unstable();
    let mut every_name: Vec<&'static str> = builder
        .generic
        .iter()
        .chain(&builder.black_ops_only)
        .copied()
        .collect();
    every_name.sort_unstable();

    SurfaceBank {
        footsteps,
        landings,
        candidates: builder.candidates,
        quiet: builder.quiet,
        world: builder.world,
        every_name,
        generic_names,
    }
}

static BANK: LazyLock<SurfaceBank> = LazyLock::new(build_bank);

/// Surface flags map to a surface index; anything out of range is "default".
fn surface_slot(surface_flags: u32) -> usize {
    let index = surface_type_index(surface_flags);
    if index < SURFACES { index } else { 0 }
}

pub fn step_prefix(gait: StepGait, local_player: bool) -> &'static str {
    match (gait, local_player) {
        (StepGait::Sprint, true) => "step_sprint_plr",
        (StepGait::Sprint, false) => "step_sprint",
        (StepGait::Run, true) => "step_run_plr",
        (StepGait::Run, false) => "step_run",
        (StepGait::Walk, true) => "step_walk_plr",
        (StepGait::Walk, false) => "step_walk",
        (StepGait::Prone, true) => "step_prone_plr",
        (StepGait::Prone, false) => "step_prone",
        (StepGait::CrouchRun, true) => "step_crouchrun_plr",
        (StepGait::CrouchRun, false) => "step_crouchrun",
        (StepGait::CrouchWalk, true) => "step_crouchwalk_plr",
        (StepGait::CrouchWalk, false) => "step_crouchwalk",
        (StepGait::LadderHand, true) => "step_ladderhand_plr",
        (StepGait::LadderHand, false) => "step_ladderhand",
    }
}

/// The footstep alias and its fallback for a gait on a surface.
pub fn footstep_aliases(
    gait: StepGait,
    surface_flags: u32,
    local_player: bool,
    quieter: bool,
) -> (&'static str, &'static str) {
    let slot = footstep_slot(gait, local_player, quieter, surface_slot(surface_flags));
    let pick = BANK.footsteps[slot];
    (pick.alias, pick.fallback)
}

/// The landing alias and its fallback for a surface.
pub fn land_aliases(
    surface_flags: u32,
    local_player: bool,
    quieter: bool,
) -> (&'static str, &'static str) {
    let pick = BANK.landings[land_slot(local_player, quieter, surface_slot(surface_flags))];
    (pick.alias, pick.fallback)
}

pub fn world_surface_alias(alias: &str) -> Option<&'static str> {
    BANK.world.get(alias).copied()
}

pub fn quiet_surface_alias(alias: &str) -> Option<&'static str> {
    BANK.quiet.get(alias).copied()
}

/// Every name worth trying for a footstep or landing alias, best first;
/// empty for an alias the bank did not generate.
pub fn surface_alias_candidates(alias: &str, _fallback: &str) -> &'static [&'static str] {
    BANK.candidates.get(alias).copied().unwrap_or(&[])
}

pub fn gear_rattle_alias(gait: StepGait, local_player: bool) -> &'static str {
    match (gait, local_player) {
        (StepGait::Sprint, true) => "gear_rattle_plr_sprint",
        (StepGait::Sprint, false) => "gear_rattle_sprint",
        // A generic bank has no crouched or ladder gear: those use the run's
        // and the walk's.
        (StepGait::Run | StepGait::CrouchRun | StepGait::LadderHand, true) => {
            "gear_rattle_plr_run"
        }
        (StepGait::Run | StepGait::CrouchRun | StepGait::LadderHand, false) => "gear_rattle_run",
        (StepGait::Walk | StepGait::CrouchWalk, true) => "gear_rattle_plr_walk",
        (StepGait::Walk | StepGait::CrouchWalk, false) => "gear_rattle_walk",
        (StepGait::Prone, true) => "gear_rattle_plr_prone",
        (StepGait::Prone, false) => "gear_rattle_prone",
    }
}

/// The gear rattle for a mantle.
pub fn gear_alias(local_player: bool) -> &'static str {
    match local_player {
        true => "gear_rattle_plr_mantle",
        false => "gear_rattle_mantle",
    }
}

/// The Black Ops II name for one of our movement aliases, if it has one.
///
/// Its footstep tables list names per surface: steps are
/// `fly_step_<gait>_<plr|npc>_<surface>`, landings `fly_land_<plr|npc>_<surface>`
/// and gear `fly_gear_<gait>_<plr|npc>`. It keeps no separate quiet set, so a
/// quiet alias maps to the plain name.
fn black_ops_movement_name(alias: &str) -> Option<String> {
    let plain = match alias.strip_prefix('q') {
        Some(rest) if rest.starts_with("step_") || rest.starts_with("Land_") => rest,
        _ => alias,
    };

    if let Some(rest) = plain.strip_prefix("step_") {
        let (gait, rest) = rest.split_once('_')?;
        // Our one-word gait names for Black Ops II's two-word ones.
        let gait = match gait {
            "crouchrun" => "crouch_run",
            "crouchwalk" => "crouch_walk",
            "ladderhand" => "ladder_hand",
            other => other,
        };
        return Some(match rest.strip_prefix("plr_") {
            Some(surface) => format!("fly_step_{gait}_plr_{surface}"),
            None => format!("fly_step_{gait}_npc_{rest}"),
        });
    }

    if let Some(rest) = plain.strip_prefix("Land_") {
        return Some(match rest.strip_prefix("plr_") {
            Some(surface) => format!("fly_land_plr_{surface}"),
            None => format!("fly_land_npc_{rest}"),
        });
    }

    let rest = alias.strip_prefix("gear_rattle_")?;
    // Another player's mantle uses Black Ops II's prone-and-mantle gear; it
    // has no fly_gear_mantle_npc.
    if rest == "mantle" {
        return Some("fly_prone_n_mantle_npc".to_owned());
    }
    Some(match rest.strip_prefix("plr_") {
        Some(gait) => format!("fly_gear_{gait}_plr"),
        None => format!("fly_gear_{rest}_npc"),
    })
}

static BLACK_OPS_NAMES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    BANK.every_name
        .iter()
        .filter_map(|&name| Some((name, leak(black_ops_movement_name(name)?))))
        .collect()
});

/// The Black Ops II name of a footstep, landing or gear sound.
pub fn t6_movement_alias(alias: &str) -> Option<&'static str> {
    BLACK_OPS_NAMES.get(alias).copied()
}

/// The movement names a generic (non-Black Ops II) bank can be asked for.
pub fn movement_prepare_names() -> &'static [&'static str] {
    &BANK.generic_names
}

/// The shot alias to use for a view: the first-person one when we are the
/// player and it is set, else the shared one. Empty names count as unset.
pub fn select_fire_alias<'a>(
    player_view: bool,
    fire: Option<&'a str>,
    fire_player: Option<&'a str>,
) -> Option<&'a str> {
    let set = |name: Option<&'a str>| name.filter(|name| !name.is_empty());
    if player_view {
        if let Some(own) = set(fire_player) {
            return Some(own);
        }
    }
    set(fire)
}

/// Like `select_fire_alias`, but the last round of a magazine prefers the
/// weapon's last-shot aliases.
pub fn select_cg_fire_alias<'a>(
    last_shot: bool,
    player_view: bool,
    fire: Option<&'a str>,
    fire_player: Option<&'a str>,
    fire_last: Option<&'a str>,
    fire_last_player: Option<&'a str>,
) -> Option<&'a str> {
    let ordinary = select_fire_alias(player_view, fire, fire_player);
    if !last_shot {
        return ordinary;
    }
    select_fire_alias(player_view, fire_last, fire_last_player).or(ordinary)
}

#[cfg(test)]
mod black_ops_name_tests {
    use super::*;

    const ASPHALT: u32 = 22 << 20;
    const CONCRETE: u32 = 5 << 20;
    const METAL: u32 = 13 << 20;

    fn step_name(gait: StepGait, flags: u32, local: bool, quieter: bool) -> Option<&'static str> {
        let (alias, _) = footstep_aliases(gait, flags, local, quieter);
        t6_movement_alias(alias)
    }

    #[test]
    fn steps_landings_and_gear_take_black_ops_names() {
        assert_eq!(
            step_name(StepGait::Run, ASPHALT, true, false),
            Some("fly_step_run_plr_asphalt")
        );
        assert_eq!(
            step_name(StepGait::Sprint, CONCRETE, false, true),
            Some("fly_step_sprint_npc_concrete")
        );

        let (land, fallback) = land_aliases(ASPHALT, true, false);
        assert_eq!(t6_movement_alias(land), Some("fly_land_plr_asphalt"));
        assert_eq!(t6_movement_alias(fallback), Some("fly_land_plr_default"));

        let sprint_gear = gear_rattle_alias(StepGait::Sprint, true);
        assert_eq!(t6_movement_alias(sprint_gear), Some("fly_gear_sprint_plr"));
        assert_eq!(t6_movement_alias(gear_alias(true)), Some("fly_gear_mantle_plr"));
    }

    #[test]
    fn crouched_and_ladder_steps_take_two_word_gait_names() {
        let (alias, fallback) = footstep_aliases(StepGait::CrouchRun, METAL, true, false);
        assert_eq!(t6_movement_alias(alias), Some("fly_step_crouch_run_plr_metal"));
        assert_eq!(
            t6_movement_alias(fallback),
            Some("fly_step_crouch_run_plr_default")
        );
        assert_eq!(
            step_name(StepGait::CrouchWalk, CONCRETE, false, false),
            Some("fly_step_crouch_walk_npc_concrete")
        );
        assert_eq!(
            step_name(StepGait::LadderHand, METAL, true, true),
            Some("fly_step_ladder_hand_plr_metal")
        );
        assert_eq!(
            step_name(StepGait::LadderHand, METAL, false, false),
            Some("fly_step_ladder_hand_npc_metal")
        );
    }

    #[test]
    fn a_generic_bank_is_never_asked_for_black_ops_only_steps() {
        let names = movement_prepare_names();
        for exclusive in ["crouchrun", "crouchwalk", "ladderhand"] {
            assert!(names.iter().all(|name| !name.contains(exclusive)));
        }
        assert!(names.contains(&"step_run_plr_metal"));

        assert_eq!(StepGait::CrouchRun.fallback(), StepGait::Run);
        assert_eq!(StepGait::CrouchWalk.fallback(), StepGait::Walk);
        assert_eq!(StepGait::LadderHand.fallback(), StepGait::Run);
        assert_eq!(StepGait::Prone.fallback(), StepGait::Prone);
    }
}
