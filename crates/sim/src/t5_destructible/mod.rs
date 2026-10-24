//! Props that break apart in stages: a Black Ops II map's cars, mannequins and loose dynents, and the older T5
//! destructible definitions. Each prop is a tree of pieces; every piece has a health and a list of stages it passes
//! through as that health drops. Leaving a stage can drop debris, play an effect or sound, and tell the map's scripts.

mod blast;
mod hide;
mod hit;
mod launch;
mod piece_damage;
mod publish;
#[cfg(test)]
mod tests;

use crate::frame::FrameWorld;
use crate::world::ClientId;
use crate::{AuthorityDObjState, AuthorityModelOwner};
use std::sync::Arc;
use xmodel_runtime::T5DestructibleDef;

pub(crate) use blast::apply_blast_by;
pub(crate) use hit::{apply_damage, apply_hit, apply_script_damage};

/// `meansOfDeath_t` numbers a prop reads that `world_objects` has no name for.
const MOD_GRENADE: u8 = 3;
const MOD_PROJECTILE: u8 = 5;
const MOD_BURNED: u8 = 17;

/// The health of every piece of one prop, plus what the map's scripts have yet to hear about it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct State {
    definition: Arc<T5DestructibleDef>,
    health: Vec<i16>,
    /// True for a Black Ops II map's prop: its breaks are queued as [`Notice`]s for the map's scripts.
    hosted: bool,
    notices: Vec<Notice>,
}

/// One thing a break tells a Black Ops II map's scripts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    /// A stage with a break notify broke (a mannequin's "headless"): `codecallback_destructibleevent("broken", ...)`.
    Broken {
        notify: String,
        attacker: Option<ClientId>,
        weapon: u32,
    },
    /// A stage that breaks by itself after `time` seconds (a burning car): the "breakafter" callback.
    BreakAfter { piece: usize, time: f32, damage: i32 },
    /// The base piece was destroyed. This ends a loose prop; BO2's own props end by their entity's health instead
    /// (see [`Notice::Damage`]).
    BaseDestroyed { attacker: Option<ClientId> },
    /// The entity itself takes `amount` (BO2's G_Damage): it hears "damage", and "death" once its health is gone.
    Damage {
        amount: i32,
        attacker: Option<ClientId>,
        means: u8,
        weapon: u32,
        point: [f32; 3],
        dir: Option<[f32; 3]>,
        flags: i32,
    },
    /// The loop the base piece's new stage plays on the entity (a burning car's fire); `None` stops it.
    Loop(Option<String>),
}

/// Which damage scale a hit counts by, and how far a piece it breaks off flies (BO2's `DestructibleDamage` and
/// `DamagePiece` take both from the means of death).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DamageKind {
    Bullet,
    /// A grenade's or projectile's own hit.
    Explosive,
    /// `MOD_EXPLOSIVE`: breaks even a stage that holds against everything else.
    Blast,
    /// A grenade's or projectile's splash: breaks past a holding stage as a blast does, but a direct hit counts as a
    /// bullet.
    Splash,
    /// Breaks only a stage that allows it.
    Melee,
    /// A thrown or fired object's impact (`MOD_IMPACT`).
    Impact,
    /// Fire: each piece takes a random share and passes none to the pieces under it.
    Burned,
    /// Any other means (a script's `dodamage`, a crush): it never sends a piece flying.
    Script,
}

/// A stage a piece left, by piece and stage index.
#[derive(Clone)]
struct Break {
    piece: usize,
    stage: usize,
}

/// What one hit did to the tree.
#[derive(Default)]
struct Outcome {
    breaks: Vec<Break>,
    /// Stages that will break on their own later: (piece, seconds, piece health).
    after: Vec<(usize, f32, i32)>,
    /// The base piece went from alive to dead.
    died: bool,
    /// The base piece changed stage: `Some(None)` is "no loop any more".
    base_loop: Option<Option<String>>,
}

/// Prepares `dobj` as a plain T5 destructible: every piece at full health.
pub fn install(dobj: &mut AuthorityDObjState, definition: Arc<T5DestructibleDef>) {
    let health = definition.pieces.iter().map(|p| p.health as i16).collect();
    dobj.t5_destructible = Some(State {
        definition,
        health,
        hosted: false,
        notices: Vec::new(),
    });
}

/// Prepares `dobj` as a Black Ops II map's destructible: full health, the bones its later stages show hidden (a
/// mannequin's broken neck), and its breaks kept for the scripts.
pub(crate) fn install_hosted(dobj: &mut AuthorityDObjState, definition: Arc<T5DestructibleDef>) {
    install(dobj, definition);
    if let Some(state) = dobj.t5_destructible.as_mut() {
        state.hosted = true;
    }
    hide::refresh_hidden_parts(dobj);
}

/// Whether `dobj` already breaks by the definition called `name`.
pub(crate) fn installed(dobj: &AuthorityDObjState, name: &str) -> bool {
    dobj.t5_destructible
        .as_ref()
        .is_some_and(|state| state.definition.name.eq_ignore_ascii_case(name))
}

/// Whether `owner` is a Black Ops II map's loose prop (a clipmap dynent named `dyn_<i>`). The client hurts those by the
/// gun's full damage, never less for range or for what the bullet went through.
pub(crate) fn is_loose_prop(world: &FrameWorld, owner: AuthorityModelOwner) -> bool {
    dobj_of(world, owner)
        .and_then(|dobj| dobj.t5_destructible.as_ref())
        .is_some_and(|state| state.definition.name.starts_with("dyn_"))
}

/// Hands over the notices the scripts have not heard yet.
pub(crate) fn take_notices(dobj: &mut AuthorityDObjState) -> Vec<Notice> {
    dobj.t5_destructible
        .as_mut()
        .map(|state| std::mem::take(&mut state.notices))
        .unwrap_or_default()
}

/// The model state of `owner`, if it has one.
fn dobj_of<'a>(world: &'a FrameWorld, owner: AuthorityModelOwner) -> Option<&'a AuthorityDObjState> {
    world
        .entity_collision_capabilities()
        .iter()
        .find(|row| row.owner == owner)
        .and_then(|row| row.dobj.as_ref())
}

fn dobj_of_mut<'a>(world: &'a mut FrameWorld, owner: AuthorityModelOwner) -> Option<&'a mut AuthorityDObjState> {
    world
        .entity_collision_capabilities_mut()
        .iter_mut()
        .find(|row| row.owner == owner)
        .and_then(|row| row.dobj.as_mut())
}

/// Queues `notice` on `owner`'s prop for the map's scripts.
fn queue_notice(world: &mut FrameWorld, owner: AuthorityModelOwner, notice: Notice) {
    if let Some(state) = dobj_of_mut(world, owner).and_then(|dobj| dobj.t5_destructible.as_mut()) {
        state.notices.push(notice);
    }
}

/// Set `BO2MP_T6_HITLOG` in the environment to log every break and blast to the diagnostics.
fn hit_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_T6_HITLOG").is_some())
}
