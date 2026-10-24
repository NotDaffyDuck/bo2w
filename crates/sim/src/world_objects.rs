//! Set pieces the server tracks apart from the entity table: the map's glass panes, the looping sounds of destructible
//! objects, and the counter telling which round of a map the glass of a client belongs to.
//!
//! The rules of BO2 glass are in `glass_t6`. This module is how the sim sees them: panes are named by piece id, a
//! pane's state becomes the damage figure that shot traces use, and shatters wait here until the scripts are told.

use crate::identities::ScriptModelId;
use glass_t6::{Cause, GlassesServer, PaneState};

pub type GlassPieceId = u32;

/// The condition of a pane, in the words its callers use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlassPieceState {
    #[default]
    Intact,
    Weakened,
    Shattered,
}

impl From<PaneState> for GlassPieceState {
    fn from(state: PaneState) -> Self {
        match state {
            PaneState::Pristine => Self::Intact,
            PaneState::Cracked => Self::Weakened,
            PaneState::Shattered => Self::Shattered,
        }
    }
}

/// Trace damage counted against a cracked pane.
pub const GLASS_DAMAGE_TO_WEAKEN: u16 = 25;

/// Trace damage from which a pane no longer stops a shot.
pub const GLASS_DAMAGE_TO_DESTROY: u16 = 100;

impl GlassPieceState {
    /// The trace damage that a pane in this state stands for.
    fn trace_damage(self) -> u16 {
        match self {
            Self::Intact => 0,
            Self::Weakened => GLASS_DAMAGE_TO_WEAKEN,
            Self::Shattered => GLASS_DAMAGE_TO_DESTROY,
        }
    }
}

/// Whether a pane that carries `damage` of trace damage still stops a shot.
pub(crate) fn glass_piece_is_solid(damage: u16) -> bool {
    damage < GLASS_DAMAGE_TO_DESTROY
}

/// Adds `added` to the damage a shot has done to a pane so far, stopping at the biggest value that fits.
pub(crate) fn glass_add_damage(total: u16, added: u32) -> u16 {
    let sum = u32::from(total).saturating_add(added);
    u16::try_from(sum).unwrap_or(u16::MAX)
}

/// What the host sends a client about these objects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldObjectSnapshot {
    pub as_of_ms: i32,
    pub map_round_epoch: u32,
    /// Every BO2 pane's state in pane order, plus the hit that broke each shattered pane. Empty without BO2 glass.
    pub t6_glass: Vec<glass_t6::PaneUpdate>,
    pub destructible_loop_sounds: Vec<DestructibleLoopSound>,
}

/// A sound that a destructible object keeps looping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DestructibleLoopSound {
    pub owner: ScriptModelId,
    pub alias_index: u8,
    pub origin: [f32; 3],
    /// Volume and pitch as hundredths of the alias's own values; 100 is the sound as authored.
    pub volume: u8,
    pub pitch: u8,
}

impl DestructibleLoopSound {
    /// Converts a scale (0 to 2.55) to hundredths, limited to the range of a byte.
    pub fn hundredths(scale: f32) -> u8 {
        (scale * 100.0).round().clamp(0.0, 255.0) as u8
    }
}

/// A BO2 glass pane as the map defines it: its place and how much it withstands.
pub type T6GlassPane = glass_t6::PaneDef;

// The glass rules and means-of-death numbers belong to `glass_t6`; callers of the sim use them under these names.
pub use glass_t6::server::{
    DAMAGE_MULTIPLIER as T6_GLASS_DAMAGE_MULTIPLIER, MELEE_DAMAGE as T6_GLASS_MELEE_DAMAGE,
    MOD_EXPLOSIVE as T6_MOD_EXPLOSIVE, MOD_GRENADE_SPLASH as T6_MOD_GRENADE_SPLASH, MOD_IMPACT as T6_MOD_IMPACT,
    MOD_MELEE as T6_MOD_MELEE, MOD_PROJECTILE_SPLASH as T6_MOD_PROJECTILE_SPLASH,
    MOD_RIFLE_BULLET as T6_MOD_RIFLE_BULLET, MOD_UNKNOWN as T6_MOD_UNKNOWN, means_from_name as t6_means_from_name,
    means_name as t6_means_name,
};

/// A shatter in the form the scripts receive it: the pane's position and the direction of the hit that broke it.
type Smash = ([f32; 3], [f32; 3]);

/// For tests: when BO2MP_T6_HITLOG is set, each pane hit and break is logged.
fn hit_log_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BO2MP_T6_HITLOG").is_some())
}

/// The map's BO2 panes, paired with the piece ids by which the rest of the sim refers to them.
#[derive(Clone, Debug, Default, PartialEq)]
struct GlassField {
    /// The piece id of each pane in ascending order: `ids[i]` belongs to `server.panes[i]`.
    ids: Vec<GlassPieceId>,
    server: GlassesServer,
    /// Shatters the scripts have not received yet (`codecallback_glasssmash`), oldest first.
    unheard: Vec<Smash>,
}

impl GlassField {
    /// The server's pane number for the piece `id`, or `None` when the piece is no pane.
    fn pane_of(&self, id: GlassPieceId) -> Option<u32> {
        let position = self.ids.binary_search(&id).ok()?;
        u32::try_from(position).ok()
    }

    fn state_of(&self, id: GlassPieceId) -> GlassPieceState {
        let pane = self.pane_of(id).and_then(|number| self.server.pane(number));
        pane.map_or(GlassPieceState::Intact, |pane| pane.state.into())
    }

    fn install(&mut self, mut panes: Vec<(GlassPieceId, T6GlassPane)>) {
        panes.sort_by_key(|&(id, _)| id);
        let (ids, defs): (Vec<_>, Vec<_>) = panes.into_iter().unzip();
        self.ids = ids;
        self.server = GlassesServer::new(defs);
        self.unheard.clear();
    }

    /// Takes the panes of another field, all of them undamaged again.
    fn copy_layout_from(&mut self, other: &Self) {
        self.ids.clone_from(&other.ids);
        let defs = other.server.panes.iter().map(|pane| pane.def);
        self.server = GlassesServer::new(defs);
    }

    /// Moves what the server recorded since the previous call into `unheard`. State changes are dropped, because
    /// clients learn states from snapshots.
    fn collect(&mut self) {
        self.server.take_changes();
        for smash in self.server.take_smashes() {
            if hit_log_enabled() {
                let id = self.ids.get(smash.pane as usize).copied().unwrap_or(u32::MAX);
                diag::info!(Sim, "bo2mp t6 glass: pane {} shattered, smash at {:?} dir {:?}", id, smash.origin, smash.dir);
            }
            self.unheard.push((smash.origin, smash.dir));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn hit(
        &mut self,
        id: GlassPieceId,
        damage: i32,
        means: u8,
        at_ms: i32,
        hit: [f32; 3],
        dir: [f32; 3],
        next_random: &mut impl FnMut() -> f32,
    ) -> GlassPieceState {
        let Some(pane) = self.pane_of(id) else {
            return GlassPieceState::Intact;
        };
        let health_before = self.server.panes[pane as usize].health;
        self.server.damage(pane, damage, means, hit, dir, at_ms, Cause::Hit, next_random);
        self.collect();
        let after = self.server.panes[pane as usize];
        if hit_log_enabled() {
            diag::info!(
                Sim,
                "bo2mp t6 glass: pane {id} hit {damage} means {means} at {at_ms} ms: health {health_before} -> {}, {:?}, bleeds {}/s",
                after.health.max(0.0),
                after.state,
                after.cracked_rate,
            );
        }
        after.state.into()
    }

    /// Lays the states of a snapshot over the panes. Health stays the host's secret: a copy only has to show states.
    fn adopt_states(&mut self, updates: &[glass_t6::PaneUpdate]) {
        self.server.reset();
        self.unheard.clear();
        for update in updates {
            let Some(pane) = self.server.panes.get_mut(usize::from(update.pane)) else {
                continue;
            };
            pane.state = update.state;
            if let Some((pos, dir)) = update.hit {
                pane.hit_pos = pos;
                pane.hit_dir = dir;
            }
            if update.state == PaneState::Shattered {
                pane.health = 0.0;
            }
        }
    }
}

/// All of the set pieces in the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldObjectState {
    loop_sounds: Vec<DestructibleLoopSound>,
    map_round_epoch: u32,
    glass: GlassField,
}

impl WorldObjectState {
    pub fn set_destructible_loop_sounds(&mut self, rows: Vec<DestructibleLoopSound>) {
        self.loop_sounds = rows;
    }

    pub fn set_map_round_epoch(&mut self, epoch: u32) {
        self.map_round_epoch = epoch;
    }

    pub fn map_round_epoch(&self) -> u32 {
        self.map_round_epoch
    }

    /// Installs the map's BO2 glass. Panes then take damage by BO2's rules, each at its full health.
    pub fn install_t6_glass(&mut self, panes: Vec<(GlassPieceId, T6GlassPane)>) {
        self.glass.install(panes);
    }

    /// Takes over the panes of another world (same map, new round).
    pub fn clone_glass_panes_from(&mut self, other: &Self) {
        self.glass.copy_layout_from(&other.glass);
    }

    pub fn has_t6_glass(&self) -> bool {
        !self.glass.ids.is_empty()
    }

    pub fn is_t6_glass(&self, id: GlassPieceId) -> bool {
        self.glass.pane_of(id).is_some()
    }

    /// The state of a pane. A piece that is not a BO2 pane reads as intact.
    pub fn glass_piece_state(&self, id: GlassPieceId) -> GlassPieceState {
        self.glass.state_of(id)
    }

    /// Whether the piece still stops shots, which every pane does until it is shattered.
    pub fn glass_is_solid(&self, id: GlassPieceId) -> bool {
        self.glass_piece_state(id) != GlassPieceState::Shattered
    }

    /// Each damaged pane with the trace damage its state stands for, in piece order; a shot's trace begins here.
    pub fn glass_damage_pairs(&self) -> Vec<(GlassPieceId, u16)> {
        let panes = self.glass.ids.iter().zip(&self.glass.server.panes);
        panes
            .filter(|(_, pane)| pane.state != PaneState::Pristine)
            .map(|(&id, pane)| (id, GlassPieceState::from(pane.state).trace_damage()))
            .collect()
    }

    /// The present health of each pane, in piece order.
    pub fn t6_glass_healths(&self) -> Vec<(GlassPieceId, f32)> {
        let panes = self.glass.ids.iter().zip(&self.glass.server.panes);
        panes.map(|(&id, pane)| (id, pane.health)).collect()
    }

    /// One hit on a pane. Grenades, projectiles, melee and impacts take all of the health; any other kind takes
    /// `damage` when it reaches 5, and less does nothing. A pane out of health shatters and the scripts are told.
    /// Otherwise the first hit cracks it, and a cracked pane loses 1 to 10 health per second until it breaks.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_t6_glass_damage(
        &mut self,
        id: GlassPieceId,
        damage: i32,
        means: u8,
        at_time_ms: i32,
        hit: [f32; 3],
        dir: [f32; 3],
        next_random: &mut impl FnMut() -> f32,
    ) -> GlassPieceState {
        self.glass.hit(id, damage, means, at_time_ms, hit, dir, next_random)
    }

    /// An explosion. Each pane inside the radius, and inside the cone when one is given, takes the blast damage for
    /// its distance: `max_damage` at the centre, falling to `min_damage` at the edge.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_t6_glass_radius(
        &mut self,
        origin: [f32; 3],
        radius: f32,
        cone: Option<([f32; 3], f32)>,
        max_damage: f32,
        min_damage: f32,
        means: u8,
        at_time_ms: i32,
        next_random: &mut impl FnMut() -> f32,
    ) {
        let server = &mut self.glass.server;
        server.radius_damage(origin, radius, cone, max_damage, min_damage, means, at_time_ms, next_random);
        self.glass.collect();
    }

    /// A scorestreak bullet hitting a pane, which BO2 handles as a rifle bullet.
    pub fn apply_glass_hit(
        &mut self,
        id: GlassPieceId,
        damage: u32,
        at_time_ms: i32,
        hit: [f32; 3],
        dir: [f32; 3],
        next_random: &mut impl FnMut() -> f32,
    ) -> GlassPieceState {
        if damage == 0 {
            return self.glass_piece_state(id);
        }
        let capped = i32::try_from(damage).unwrap_or(i32::MAX);
        self.apply_t6_glass_damage(id, capped, T6_MOD_RIFLE_BULLET, at_time_ms, hit, dir, next_random)
    }

    /// Breaks a pane immediately, like a hit that takes its whole health. A pane that is already broken stays so.
    pub fn force_shatter_glass(
        &mut self,
        id: GlassPieceId,
        at_time_ms: i32,
        hit: [f32; 3],
        dir: [f32; 3],
    ) -> GlassPieceState {
        if !self.glass_is_solid(id) {
            return GlassPieceState::Shattered;
        }
        self.glass.hit(id, 0, T6_MOD_IMPACT, at_time_ms, hit, dir, &mut || 0.0)
    }

    /// A script's `destroy` of a pane: it breaks where it stands and the pieces go upward.
    pub fn script_destroy_glass(&mut self, id: GlassPieceId, at_time_ms: i32) -> GlassPieceState {
        let pane = self.glass.pane_of(id).and_then(|number| self.glass.server.pane(number));
        let origin = pane.map_or([0.0; 3], |pane| pane.def.origin);
        self.force_shatter_glass(id, at_time_ms, origin, [0.0, 0.0, 1.0])
    }

    /// Lets cracked panes bleed until `at_time_ms`; a pane that runs out of health breaks on its own.
    pub fn glass_update(&mut self, at_time_ms: i32) {
        if !self.has_t6_glass() {
            return;
        }
        self.glass.server.update(at_time_ms);
        self.glass.collect();
    }

    /// The shatters since the previous call, for `codecallback_glasssmash`.
    pub fn take_glass_smashes(&mut self) -> Vec<([f32; 3], [f32; 3])> {
        std::mem::take(&mut self.glass.unheard)
    }

    pub fn to_snapshot(&self) -> WorldObjectSnapshot {
        WorldObjectSnapshot {
            as_of_ms: 0,
            map_round_epoch: self.map_round_epoch,
            t6_glass: glass_t6::net::full_list(&self.glass.server.panes),
            destructible_loop_sounds: self.loop_sounds.clone(),
        }
    }

    pub fn adopt_snapshot(&mut self, snap: &WorldObjectSnapshot) {
        self.map_round_epoch = snap.map_round_epoch;
        self.loop_sounds.clone_from(&snap.destructible_loop_sounds);
        self.glass.adopt_states(&snap.t6_glass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const FULL: f32 = 120.0;

    /// A pane like the one in Raid's `glass_dec_clear_mp`: 120 health, a point as its box.
    fn pane_at(origin: [f32; 3]) -> T6GlassPane {
        T6GlassPane {
            origin,
            absmin: origin,
            absmax: origin,
            max_health: FULL,
        }
    }

    /// A world with panes at `origins`, numbered from 0.
    fn world_with_panes(origins: &[[f32; 3]]) -> WorldObjectState {
        let mut world = WorldObjectState::default();
        let panes = origins.iter().zip(0..).map(|(&origin, id)| (id, pane_at(origin)));
        world.install_t6_glass(panes.collect());
        world
    }

    fn health_of(world: &WorldObjectState, id: GlassPieceId) -> f32 {
        world
            .t6_glass_healths()
            .into_iter()
            .find_map(|(piece, health)| (piece == id).then_some(health))
            .expect("the pane exists")
    }

    /// A hit on the pane eastward from (1, 2, 3) at 1000 ms, with the crack-rate roll fixed at one half.
    fn hit_pane(world: &mut WorldObjectState, id: GlassPieceId, damage: i32, means: u8) -> GlassPieceState {
        world.apply_t6_glass_damage(id, damage, means, 1000, [1.0, 2.0, 3.0], EAST, &mut || 0.5)
    }

    /// Advances the glass clock in 50 ms ticks until it reaches `until_ms`.
    fn advance_to(world: &mut WorldObjectState, clock_ms: &mut i32, until_ms: i32) {
        while *clock_ms < until_ms {
            *clock_ms += 50;
            world.glass_update(*clock_ms);
        }
    }

    #[test]
    fn a_weak_hit_does_nothing_and_a_firm_one_cracks() {
        let mut world = world_with_panes(&[[10.0, 20.0, 30.0]]);
        assert_eq!(hit_pane(&mut world, 0, 4, T6_MOD_RIFLE_BULLET), GlassPieceState::Intact);
        assert_eq!(health_of(&world, 0), FULL);
        assert_eq!(hit_pane(&mut world, 0, 30, T6_MOD_RIFLE_BULLET), GlassPieceState::Weakened);
        assert_eq!(health_of(&world, 0), 90.0);
        assert!(world.take_glass_smashes().is_empty());
    }

    #[test]
    fn a_pane_without_health_shatters_and_is_reported_once() {
        let mut world = world_with_panes(&[[10.0, 20.0, 30.0]]);
        hit_pane(&mut world, 0, 30, T6_MOD_RIFLE_BULLET);
        assert_eq!(hit_pane(&mut world, 0, 90, T6_MOD_RIFLE_BULLET), GlassPieceState::Shattered);
        assert_eq!(world.take_glass_smashes(), vec![([10.0, 20.0, 30.0], EAST)]);
        assert!(world.take_glass_smashes().is_empty());
        // Once broken it takes no further hits and reports nothing more.
        hit_pane(&mut world, 0, 90, T6_MOD_RIFLE_BULLET);
        assert!(world.take_glass_smashes().is_empty());
    }

    #[test]
    fn some_kinds_of_damage_break_a_pane_in_a_single_hit() {
        for means in [3, 5, T6_MOD_MELEE, 8, T6_MOD_IMPACT, 17] {
            let mut world = world_with_panes(&[[0.0; 3]]);
            assert_eq!(hit_pane(&mut world, 0, 1, means), GlassPieceState::Shattered, "means {means}");
        }
    }

    #[test]
    fn grenade_splash_counts_as_ordinary_damage() {
        let mut world = world_with_panes(&[[0.0; 3]]);
        assert_eq!(hit_pane(&mut world, 0, 40, T6_MOD_GRENADE_SPLASH), GlassPieceState::Weakened);
        assert_eq!(health_of(&world, 0), 80.0);
    }

    #[test]
    fn a_cracked_pane_bleeds_until_it_breaks_by_itself() {
        let mut world = world_with_panes(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        // A roll of one half gives 1 + 9 x 0.5 = 5.5 health per second, so 90 health lasts about 16.4 seconds.
        hit_pane(&mut world, 0, 30, T6_MOD_RIFLE_BULLET);
        let mut clock = 1000;
        world.glass_update(clock);
        advance_to(&mut world, &mut clock, 1000 + 16_300);
        assert_eq!(world.glass_piece_state(0), GlassPieceState::Weakened);
        advance_to(&mut world, &mut clock, 1000 + 16_500);
        assert_eq!(world.glass_piece_state(0), GlassPieceState::Shattered);
        // Having bled out, the pane had no hit direction to fall along.
        assert_eq!(world.take_glass_smashes(), vec![([0.0; 3], [0.0; 3])]);
        assert_eq!(world.glass_piece_state(1), GlassPieceState::Intact);
        assert_eq!(health_of(&world, 1), FULL);
    }

    #[test]
    fn an_explosion_weakens_with_distance() {
        let mut world = world_with_panes(&[[100.0, 0.0, 0.0], [250.0, 0.0, 0.0]]);
        world.apply_t6_glass_radius([0.0; 3], 200.0, None, 100.0, 20.0, T6_MOD_EXPLOSIVE, 1000, &mut || 0.5);
        // At half the radius the damage is 20 + (100 - 20) x 0.5 = 60.
        assert_eq!(health_of(&world, 0), 60.0);
        assert_eq!(world.glass_piece_state(0), GlassPieceState::Weakened);
        assert_eq!(health_of(&world, 1), FULL);
    }

    #[test]
    fn an_explosion_spares_panes_outside_its_cone() {
        let mut world = world_with_panes(&[[100.0, 0.0, 0.0]]);
        let cone = Some(([0.0, 1.0, 0.0], 0.5));
        world.apply_t6_glass_radius([0.0; 3], 200.0, cone, 100.0, 20.0, T6_MOD_EXPLOSIVE, 1000, &mut || 0.5);
        assert_eq!(health_of(&world, 0), FULL);
    }

    #[test]
    fn a_pane_broken_by_an_explosion_is_reported_to_the_scripts() {
        let mut world = world_with_panes(&[[10.0, 0.0, 0.0]]);
        world.apply_t6_glass_radius([0.0; 3], 200.0, None, 500.0, 500.0, T6_MOD_EXPLOSIVE, 1000, &mut || 0.5);
        assert_eq!(world.glass_piece_state(0), GlassPieceState::Shattered);
        assert_eq!(world.take_glass_smashes().len(), 1);
    }

    #[test]
    fn a_scripted_destroy_breaks_the_pane_in_place_with_pieces_flying_up() {
        let mut world = world_with_panes(&[[5.0, 6.0, 7.0]]);
        assert_eq!(world.script_destroy_glass(0, 1000), GlassPieceState::Shattered);
        assert_eq!(world.take_glass_smashes(), vec![([5.0, 6.0, 7.0], [0.0, 0.0, 1.0])]);
        // A second destroy neither changes the pane nor reports anything.
        assert_eq!(world.script_destroy_glass(0, 2000), GlassPieceState::Shattered);
        assert!(world.take_glass_smashes().is_empty());
    }

    #[test]
    fn a_forced_shatter_uses_the_hit_it_was_given() {
        let mut world = world_with_panes(&[[5.0, 6.0, 7.0]]);
        world.force_shatter_glass(0, 1000, [9.0, 9.0, 9.0], EAST);
        assert_eq!(world.take_glass_smashes(), vec![([5.0, 6.0, 7.0], EAST)]);
        assert_eq!(world.glass_damage_pairs(), vec![(0, GLASS_DAMAGE_TO_DESTROY)]);
    }

    #[test]
    fn pieces_that_are_no_pane_are_left_alone() {
        let mut world = WorldObjectState::default();
        world.install_t6_glass(vec![(7, pane_at([0.0; 3]))]);
        assert!(world.is_t6_glass(7));
        assert!(!world.is_t6_glass(3));
        assert_eq!(world.glass_piece_state(3), GlassPieceState::Intact);
        assert!(world.glass_is_solid(3));
        assert_eq!(hit_pane(&mut world, 3, 500, T6_MOD_MELEE), GlassPieceState::Intact);
        assert_eq!(world.force_shatter_glass(3, 1000, [0.0; 3], EAST), GlassPieceState::Intact);
        assert_eq!(world.script_destroy_glass(3, 1000), GlassPieceState::Intact);
        assert_eq!(health_of(&world, 7), FULL);
        assert!(world.take_glass_smashes().is_empty());
    }

    #[test]
    fn installed_panes_end_up_sorted_by_piece_id() {
        let mut world = WorldObjectState::default();
        assert!(!world.has_t6_glass());
        world.install_t6_glass(vec![
            (9, pane_at([9.0, 0.0, 0.0])),
            (2, pane_at([2.0, 0.0, 0.0])),
            (5, pane_at([5.0, 0.0, 0.0])),
        ]);
        assert!(world.has_t6_glass());
        let ids: Vec<_> = world.t6_glass_healths().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [2, 5, 9]);
        hit_pane(&mut world, 5, 30, T6_MOD_RIFLE_BULLET);
        assert_eq!(world.glass_damage_pairs(), vec![(5, GLASS_DAMAGE_TO_WEAKEN)]);
        assert_eq!(health_of(&world, 2), FULL);
        assert_eq!(health_of(&world, 9), FULL);
    }

    #[test]
    fn the_next_round_starts_with_the_same_panes_undamaged() {
        let mut played = world_with_panes(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        hit_pane(&mut played, 0, 30, T6_MOD_RIFLE_BULLET);
        hit_pane(&mut played, 1, 1, T6_MOD_MELEE);
        let mut next = WorldObjectState::default();
        next.clone_glass_panes_from(&played);
        assert_eq!(next.t6_glass_healths(), vec![(0, FULL), (1, FULL)]);
        assert!(next.glass_damage_pairs().is_empty());
    }

    #[test]
    fn scorestreak_bullets_hit_panes_as_rifle_bullets() {
        let mut world = world_with_panes(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        let mut shoot = |id, damage| world.apply_glass_hit(id, damage, 1000, [0.0; 3], EAST, &mut || 0.5);
        assert_eq!(shoot(0, 0), GlassPieceState::Intact);
        assert_eq!(shoot(0, 30), GlassPieceState::Weakened);
        assert_eq!(shoot(1, u32::MAX), GlassPieceState::Shattered);
    }

    #[test]
    fn the_glass_clock_is_idle_without_bo2_glass() {
        let mut world = WorldObjectState::default();
        world.glass_update(1000);
        world.glass_update(2000);
        assert!(world.t6_glass_healths().is_empty());
        assert!(world.take_glass_smashes().is_empty());
    }

    #[test]
    fn a_client_copy_takes_over_the_states_in_the_host_snapshot() {
        let mut host = world_with_panes(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        let mut client = world_with_panes(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        hit_pane(&mut host, 0, 1, T6_MOD_MELEE);
        host.apply_glass_hit(1, 30, 1000, [64.0, 0.0, 0.0], EAST, &mut || 0.5);
        client.adopt_snapshot(&host.to_snapshot());
        assert!(!client.glass_is_solid(0));
        assert_eq!(client.glass_piece_state(1), GlassPieceState::Weakened);
        assert!(client.glass_is_solid(1));
        let expected = vec![(0, GLASS_DAMAGE_TO_DESTROY), (1, GLASS_DAMAGE_TO_WEAKEN)];
        assert_eq!(client.glass_damage_pairs(), expected);
        assert!(client.take_glass_smashes().is_empty());
    }

    #[test]
    fn the_snapshot_also_holds_the_round_and_the_loop_sounds() {
        let sound = DestructibleLoopSound {
            owner: ScriptModelId::from_wire(4),
            alias_index: 2,
            origin: [1.0, 2.0, 3.0],
            volume: 100,
            pitch: 50,
        };
        let mut host = WorldObjectState::default();
        host.set_map_round_epoch(3);
        host.set_destructible_loop_sounds(vec![sound]);
        let snapshot = host.to_snapshot();
        assert_eq!((snapshot.as_of_ms, snapshot.map_round_epoch), (0, 3));
        assert_eq!(snapshot.destructible_loop_sounds, vec![sound]);
        let mut client = WorldObjectState::default();
        client.adopt_snapshot(&snapshot);
        assert_eq!(client.map_round_epoch(), 3);
        assert_eq!(client.to_snapshot(), snapshot);
    }

    #[test]
    fn a_trace_is_stopped_by_a_pane_only_under_the_destroy_damage() {
        assert!(glass_piece_is_solid(0));
        assert!(glass_piece_is_solid(GLASS_DAMAGE_TO_DESTROY - 1));
        assert!(!glass_piece_is_solid(GLASS_DAMAGE_TO_DESTROY));
    }

    #[test]
    fn damage_to_a_pane_adds_up_and_saturates() {
        assert_eq!(glass_add_damage(10, 5), 15);
        assert_eq!(glass_add_damage(65_530, 10), u16::MAX);
        assert_eq!(glass_add_damage(1, u32::MAX), u16::MAX);
    }

    #[test]
    fn loop_scales_turn_into_byte_sized_hundredths() {
        let cases = [(0.0, 0), (0.5, 50), (1.0, 100), (-3.0, 0), (9.0, 255)];
        for (scale, expected) in cases {
            assert_eq!(DestructibleLoopSound::hundredths(scale), expected, "scale {scale}");
        }
    }

    #[test]
    fn means_of_death_names_match_the_bo2_table() {
        assert_eq!(t6_means_from_name("MOD_RIFLE_BULLET"), T6_MOD_RIFLE_BULLET);
        assert_eq!(t6_means_from_name("mod_melee"), T6_MOD_MELEE);
        assert_eq!(t6_means_from_name("MOD_EXPLOSIVE"), T6_MOD_EXPLOSIVE);
        assert_eq!(t6_means_from_name("MOD_GAS"), 20);
        assert_eq!(t6_means_from_name("MOD_NOT_A_KIND"), T6_MOD_UNKNOWN);
        assert_eq!(t6_means_name(T6_MOD_EXPLOSIVE), "MOD_EXPLOSIVE");
        assert_eq!(t6_means_name(200), "MOD_UNKNOWN");
    }
}
