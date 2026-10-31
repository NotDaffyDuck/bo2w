//! The map's glass as the sim sees it. The rules of BO2 glass live in `glass_t6`; here the panes get piece ids, a
//! pane's state turns into the trace damage that shots count against it, and shatters wait until the scripts hear.

use glass_t6::{Cause, GlassesServer, PaneState};

/// How intact a pane is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneShardCondition {
    #[default]
    Intact,
    Weakened,
    Shattered,
}

impl From<PaneState> for PaneShardCondition {
    fn from(raw: PaneState) -> Self {
        match raw {
            PaneState::Pristine => Self::Intact,
            PaneState::Cracked => Self::Weakened,
            PaneState::Shattered => Self::Shattered,
        }
    }
}

/// The trace damage a cracked pane stands for.
pub const PANE_CRACK_HARM: u16 = 25;

/// From this much trace damage on, a pane no longer stops a shot.
pub const PANE_SHATTER_HARM: u16 = 100;

impl PaneShardCondition {
    /// The trace damage a pane in this state counts for.
    fn trace_damage(self) -> u16 {
        match self {
            Self::Intact => 0,
            Self::Weakened => PANE_CRACK_HARM,
            Self::Shattered => PANE_SHATTER_HARM,
        }
    }
}

/// Whether a pane carrying `trace_damage` of trace damage still stops a shot.
pub(crate) fn shard_blocks_shots(trace_damage: u16) -> bool {
    trace_damage < PANE_SHATTER_HARM
}

/// The trace damage `total` grows to when `more` lands on it, held at the largest value that fits.
pub(crate) fn pile_harm(total: u16, more: u32) -> u16 {
    u16::try_from(u32::from(total).saturating_add(more)).unwrap_or(u16::MAX)
}

pub use glass_t6::server::{
    DAMAGE_MULTIPLIER as T6_GLASS_DAMAGE_MULTIPLIER, MELEE_DAMAGE as T6_GLASS_MELEE_DAMAGE,
    MOD_EXPLOSIVE as T6_MOD_EXPLOSIVE, MOD_GRENADE_SPLASH as T6_MOD_GRENADE_SPLASH, MOD_IMPACT as T6_MOD_IMPACT,
    MOD_MELEE as T6_MOD_MELEE, MOD_PROJECTILE_SPLASH as T6_MOD_PROJECTILE_SPLASH,
    MOD_RIFLE_BULLET as T6_MOD_RIFLE_BULLET, MOD_UNKNOWN as T6_MOD_UNKNOWN, means_from_name as t6_means_from_name,
    means_name as t6_means_name,
};

/// The spot where something struck a pane and the way it was heading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Strike {
    pub at: [f32; 3],
    pub way: [f32; 3],
}

/// One hit on one pane. Grenades, launched rounds, melee and impacts take all of a pane's health; any other kind takes
/// `damage` once that reaches 5, and less does nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blow {
    pub pane: u32,
    pub harm: i32,
    pub means: u8,
    pub at_ms: i32,
    pub strike: Strike,
}

impl Blow {
    /// A scorestreak bullet; BO2 handles those as rifle bullets.
    pub fn bullet(pane: u32, harm: u32, at_ms: i32, strike: Strike) -> Self {
        let damage = i32::try_from(harm).unwrap_or(i32::MAX);
        Self { pane, harm: damage, means: T6_MOD_RIFLE_BULLET, at_ms, strike }
    }
}

/// An explosion's reach over the glass: `max_damage` at the centre, falling to `min_damage` at the radius, and only
/// inside the cone (its axis and the cosine of its half angle) when there is one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub spot: [f32; 3],
    pub reach: f32,
    pub cone: Option<([f32; 3], f32)>,
    pub max_damage: f32,
    pub min_damage: f32,
    pub means: u8,
}

fn hit_log_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BO2MP_T6_HITLOG").is_some())
}

/// The map's panes with the piece ids they go by.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct GlassField {
    /// Ascending piece ids; `ids[n]` is the id of the server's pane `n`.
    ids: Vec<u32>,
    server: GlassesServer,
    /// Shatters the scripts have not heard about yet, oldest first.
    unheard: Vec<([f32; 3], [f32; 3])>,
}

impl GlassField {
    pub(super) fn has_no_panes(&self) -> bool {
        self.ids.is_empty()
    }

    /// The server's number for the pane with piece id `id`, if there is one.
    fn number_of(&self, id: u32) -> Option<u32> {
        let at = self.ids.binary_search(&id).ok()?;
        u32::try_from(at).ok()
    }

    pub(super) fn is_pane(&self, id: u32) -> bool {
        self.number_of(id).is_some()
    }

    pub(super) fn state_of(&self, id: u32) -> PaneShardCondition {
        let pane = self.number_of(id).and_then(|number| self.server.pane(number));
        pane.map_or(PaneShardCondition::Intact, |pane| pane.state.into())
    }

    pub(super) fn install(&mut self, mut panes: Vec<(u32, glass_t6::PaneDef)>) {
        panes.sort_by_key(|&(id, _)| id);
        let (ids, defs): (Vec<_>, Vec<_>) = panes.into_iter().unzip();
        self.ids = ids;
        self.server = GlassesServer::new(defs);
        self.unheard.clear();
    }

    /// Takes over the panes of `source`, every one undamaged.
    pub(super) fn reset_to_layout_of(&mut self, source: &Self) {
        self.ids.clone_from(&source.ids);
        self.server = GlassesServer::new(source.server.panes.iter().map(|pane| pane.def));
    }

    /// Files what the server recorded since the last call: its shatters wait for the scripts, its state changes go,
    /// since clients learn states from snapshots.
    fn file_reports(&mut self) {
        self.server.take_changes();
        for smash in self.server.take_smashes() {
            if hit_log_enabled() {
                let id = self.ids.get(smash.pane as usize).copied().unwrap_or(u32::MAX);
                diag::info!(Sim, "glass pane {} broke: pieces fly from {:?} along {:?}", id, smash.origin, smash.facing);
            }
            self.unheard.push((smash.origin, smash.facing));
        }
    }

    pub(super) fn strike(&mut self, blow: Blow, mut roll: &mut dyn FnMut() -> f32) -> PaneShardCondition {
        let Some(number) = self.number_of(blow.pane) else {
            return PaneShardCondition::Intact;
        };
        let before = self.server.panes[number as usize].health;
        let Blow { harm: damage, means, at_ms, strike, .. } = blow;
        self.server.damage(number, damage, means, strike.at, strike.way, at_ms, Cause::Hit, &mut roll);
        self.file_reports();
        let pane = self.server.panes[number as usize];
        if hit_log_enabled() {
            diag::info!(
                Sim,
                "glass pane {} took {damage} ({means}) at {at_ms} ms, health now {} (was {before}), {:?}, losing {} a second",
                blow.pane,
                pane.health.max(0.0),
                pane.state,
                pane.cracked_rate,
            );
        }
        pane.state.into()
    }

    pub(super) fn blast(&mut self, blast: &Blast, at_ms: i32, mut roll: &mut dyn FnMut() -> f32) {
        let Blast { spot: origin, reach: radius, cone, max_damage, min_damage, means } = *blast;
        self.server.radius_damage(origin, radius, cone, max_damage, min_damage, means, at_ms, &mut roll);
        self.file_reports();
    }

    /// Breaks the pane at once, as a hit that takes all of its health. A pane already broken stays so.
    pub(super) fn shatter(&mut self, id: u32, at_ms: i32, strike: Strike) -> PaneShardCondition {
        let already_broken = self.state_of(id) == PaneShardCondition::Shattered;
        if already_broken {
            PaneShardCondition::Shattered
        } else {
            self.strike(Blow { pane: id, harm: 0, means: T6_MOD_IMPACT, at_ms, strike }, &mut || 0.0)
        }
    }

    /// A script's `destroy`: the pane breaks where it stands and its pieces go upward.
    pub(super) fn destroy(&mut self, id: u32, at_ms: i32) -> PaneShardCondition {
        let pane = self.number_of(id).and_then(|number| self.server.pane(number));
        let Some(pane) = pane else {
            return PaneShardCondition::Intact;
        };
        let strike = Strike { at: pane.def.origin, way: [0.0, 0.0, 1.0] };
        self.shatter(id, at_ms, strike)
    }

    /// Lets cracked panes bleed up to `at_ms`; one that runs out of health breaks by itself.
    pub(super) fn advance(&mut self, at_ms: i32) {
        if self.has_no_panes() {
            return;
        }
        self.server.update(at_ms);
        self.file_reports();
    }

    pub(super) fn take_smashes(&mut self) -> Vec<([f32; 3], [f32; 3])> {
        std::mem::take(&mut self.unheard)
    }

    /// Each damaged pane with the trace damage its state stands for, in piece order.
    pub(super) fn trace_damage(&self) -> Vec<(u32, u16)> {
        let panes = self.ids.iter().zip(&self.server.panes);
        panes
            .filter(|pair| pair.1.state != PaneState::Pristine)
            .map(|(&id, pane)| (id, PaneShardCondition::from(pane.state).trace_damage()))
            .collect()
    }

    /// Every pane's health in piece order.
    pub(super) fn healths(&self) -> Vec<(u32, f32)> {
        self.ids.iter().zip(&self.server.panes).map(|(&id, pane)| (id, pane.health)).collect()
    }

    pub(super) fn wire_list(&self) -> Vec<glass_t6::PaneUpdate> {
        glass_t6::net::full_list(&self.server.panes)
    }

    /// Lays the states of a snapshot over the panes. Health stays the host's secret: a copy only shows states.
    pub(super) fn adopt_states(&mut self, updates: &[glass_t6::PaneUpdate]) {
        self.server.reset();
        self.unheard.clear();
        for update in updates {
            let Some(pane) = self.server.panes.get_mut(usize::from(update.pane)) else {
                continue;
            };
            pane.state = update.state;
            if let Some((at, dir)) = update.hit {
                pane.impact_point = at;
                pane.hit_dir = dir;
            }
            if update.state == PaneState::Shattered {
                pane.health = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const FULL: f32 = 120.0;

    /// A pane like the one in Raid's `glass_dec_clear_mp`: 120 health, a point as its box.
    fn pane_at(origin: [f32; 3]) -> glass_t6::PaneDef {
        glass_t6::PaneDef { origin, absmin: origin, absmax: origin, hit_point_ceiling: FULL }
    }

    /// Panes at `origins`, numbered from 0.
    fn field_of(origins: &[[f32; 3]]) -> GlassField {
        let mut field = GlassField::default();
        field.install(origins.iter().zip(0..).map(|(&origin, id)| (id, pane_at(origin))).collect());
        field
    }

    fn health_of(field: &GlassField, id: u32) -> f32 {
        field.healths().into_iter().find_map(|(piece, health)| (piece == id).then_some(health)).expect("a pane")
    }

    /// A hit on pane `id` going east from (1, 2, 3) at 1000 ms, the crack-rate roll fixed at one half.
    fn hit(field: &mut GlassField, id: u32, harm: i32, means: u8) -> PaneShardCondition {
        let strike = Strike { at: [1.0, 2.0, 3.0], way: EAST };
        field.strike(Blow { pane: id, harm: harm, means, at_ms: 1000, strike }, &mut || 0.5)
    }

    /// Runs the glass clock in 50 ms ticks from `*clock` up to `until`.
    fn run_clock(field: &mut GlassField, clock: &mut i32, until: i32) {
        while *clock < until {
            *clock += 50;
            field.advance(*clock);
        }
    }

    #[test]
    fn a_weak_hit_does_nothing_and_a_firm_one_cracks() {
        let mut field = field_of(&[[10.0, 20.0, 30.0]]);
        assert_eq!(hit(&mut field, 0, 4, T6_MOD_RIFLE_BULLET), PaneShardCondition::Intact);
        assert_eq!(health_of(&field, 0), FULL);
        assert_eq!(hit(&mut field, 0, 30, T6_MOD_RIFLE_BULLET), PaneShardCondition::Weakened);
        assert_eq!(health_of(&field, 0), 90.0);
        assert!(field.take_smashes().is_empty());
    }

    #[test]
    fn a_pane_without_health_shatters_and_is_heard_once() {
        let mut field = field_of(&[[10.0, 20.0, 30.0]]);
        hit(&mut field, 0, 30, T6_MOD_RIFLE_BULLET);
        assert_eq!(hit(&mut field, 0, 90, T6_MOD_RIFLE_BULLET), PaneShardCondition::Shattered);
        assert_eq!(field.take_smashes(), vec![([10.0, 20.0, 30.0], EAST)]);
        assert!(field.take_smashes().is_empty());
        // A broken pane takes no further hits and says nothing more.
        hit(&mut field, 0, 90, T6_MOD_RIFLE_BULLET);
        assert!(field.take_smashes().is_empty());
    }

    #[test]
    fn some_kinds_of_damage_break_a_pane_in_one_hit() {
        for means in [3, 5, T6_MOD_MELEE, 8, T6_MOD_IMPACT, 17] {
            let mut field = field_of(&[[0.0; 3]]);
            assert_eq!(hit(&mut field, 0, 1, means), PaneShardCondition::Shattered, "means {means}");
        }
    }

    #[test]
    fn grenade_splash_counts_as_ordinary_damage() {
        let mut field = field_of(&[[0.0; 3]]);
        assert_eq!(hit(&mut field, 0, 40, T6_MOD_GRENADE_SPLASH), PaneShardCondition::Weakened);
        assert_eq!(health_of(&field, 0), 80.0);
    }

    #[test]
    fn a_cracked_pane_bleeds_until_it_breaks_by_itself() {
        let mut field = field_of(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        // A roll of one half gives 1 + 9 x 0.5 = 5.5 health per second, so 90 health lasts about 16.4 seconds.
        hit(&mut field, 0, 30, T6_MOD_RIFLE_BULLET);
        let mut clock = 1000;
        field.advance(clock);
        run_clock(&mut field, &mut clock, 1000 + 16_300);
        assert_eq!(field.state_of(0), PaneShardCondition::Weakened);
        run_clock(&mut field, &mut clock, 1000 + 16_500);
        assert_eq!(field.state_of(0), PaneShardCondition::Shattered);
        // Bled out, the pane had no hit direction to fall along.
        assert_eq!(field.take_smashes(), vec![([0.0; 3], [0.0; 3])]);
        assert_eq!(field.state_of(1), PaneShardCondition::Intact);
        assert_eq!(health_of(&field, 1), FULL);
    }

    fn explosion(origin: [f32; 3], cone: Option<([f32; 3], f32)>, max_damage: f32, min_damage: f32) -> Blast {
        Blast { spot: origin, reach: 200.0, cone, max_damage, min_damage, means: T6_MOD_EXPLOSIVE }
    }

    #[test]
    fn an_explosion_weakens_with_distance() {
        let mut field = field_of(&[[100.0, 0.0, 0.0], [250.0, 0.0, 0.0]]);
        field.blast(&explosion([0.0; 3], None, 100.0, 20.0), 1000, &mut || 0.5);
        // At half the radius the damage is 20 + (100 - 20) x 0.5 = 60.
        assert_eq!(health_of(&field, 0), 60.0);
        assert_eq!(field.state_of(0), PaneShardCondition::Weakened);
        assert_eq!(health_of(&field, 1), FULL);
    }

    #[test]
    fn an_explosion_spares_panes_outside_its_cone() {
        let mut field = field_of(&[[100.0, 0.0, 0.0]]);
        field.blast(&explosion([0.0; 3], Some(([0.0, 1.0, 0.0], 0.5)), 100.0, 20.0), 1000, &mut || 0.5);
        assert_eq!(health_of(&field, 0), FULL);
    }

    #[test]
    fn a_pane_broken_by_an_explosion_is_heard() {
        let mut field = field_of(&[[10.0, 0.0, 0.0]]);
        field.blast(&explosion([0.0; 3], None, 500.0, 500.0), 1000, &mut || 0.5);
        assert_eq!(field.state_of(0), PaneShardCondition::Shattered);
        assert_eq!(field.take_smashes().len(), 1);
    }

    #[test]
    fn a_scripted_destroy_breaks_the_pane_in_place_with_pieces_flying_up() {
        let mut field = field_of(&[[5.0, 6.0, 7.0]]);
        assert_eq!(field.destroy(0, 1000), PaneShardCondition::Shattered);
        assert_eq!(field.take_smashes(), vec![([5.0, 6.0, 7.0], [0.0, 0.0, 1.0])]);
        // Destroying it again changes nothing and says nothing.
        assert_eq!(field.destroy(0, 2000), PaneShardCondition::Shattered);
        assert!(field.take_smashes().is_empty());
    }

    #[test]
    fn a_forced_shatter_goes_the_way_it_was_struck() {
        let mut field = field_of(&[[5.0, 6.0, 7.0]]);
        field.shatter(0, 1000, Strike { at: [9.0; 3], way: EAST });
        assert_eq!(field.take_smashes(), vec![([5.0, 6.0, 7.0], EAST)]);
        assert_eq!(field.trace_damage(), vec![(0, PANE_SHATTER_HARM)]);
    }

    #[test]
    fn pieces_that_are_no_pane_are_left_alone() {
        let mut field = GlassField::default();
        field.install(vec![(7, pane_at([0.0; 3]))]);
        assert!(field.is_pane(7));
        assert!(!field.is_pane(3));
        assert_eq!(field.state_of(3), PaneShardCondition::Intact);
        assert_eq!(hit(&mut field, 3, 500, T6_MOD_MELEE), PaneShardCondition::Intact);
        assert_eq!(field.shatter(3, 1000, Strike::default()), PaneShardCondition::Intact);
        assert_eq!(field.destroy(3, 1000), PaneShardCondition::Intact);
        assert_eq!(health_of(&field, 7), FULL);
        assert!(field.take_smashes().is_empty());
    }

    #[test]
    fn installed_panes_are_kept_in_piece_order() {
        let mut field = GlassField::default();
        assert!(field.has_no_panes());
        field.install(vec![(9, pane_at([9.0; 3])), (2, pane_at([2.0; 3])), (5, pane_at([5.0; 3]))]);
        assert!(!field.has_no_panes());
        let ids: Vec<_> = field.healths().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [2, 5, 9]);
        hit(&mut field, 5, 30, T6_MOD_RIFLE_BULLET);
        assert_eq!(field.trace_damage(), vec![(5, PANE_CRACK_HARM)]);
        assert_eq!(health_of(&field, 2), FULL);
        assert_eq!(health_of(&field, 9), FULL);
    }

    #[test]
    fn the_next_round_has_the_same_panes_undamaged() {
        let mut played = field_of(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        hit(&mut played, 0, 30, T6_MOD_RIFLE_BULLET);
        hit(&mut played, 1, 1, T6_MOD_MELEE);
        let mut next = GlassField::default();
        next.reset_to_layout_of(&played);
        assert_eq!(next.healths(), vec![(0, FULL), (1, FULL)]);
        assert!(next.trace_damage().is_empty());
    }

    #[test]
    fn scorestreak_bullets_hit_panes_as_rifle_bullets() {
        let mut field = field_of(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        let mut shoot = |id, damage| {
            let strike = Strike { at: [0.0; 3], way: EAST };
            field.strike(Blow::bullet(id, damage, 1000, strike), &mut || 0.5)
        };
        assert_eq!(shoot(0, 0), PaneShardCondition::Intact);
        assert_eq!(shoot(0, 30), PaneShardCondition::Weakened);
        assert_eq!(shoot(1, u32::MAX), PaneShardCondition::Shattered);
    }

    #[test]
    fn an_idle_field_ignores_the_clock() {
        let mut field = GlassField::default();
        field.advance(1000);
        field.advance(2000);
        assert!(field.healths().is_empty());
        assert!(field.take_smashes().is_empty());
    }

    #[test]
    fn a_copy_shows_the_states_of_the_host_snapshot() {
        let mut host = field_of(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        let mut copy = field_of(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        hit(&mut host, 0, 1, T6_MOD_MELEE);
        hit(&mut host, 1, 30, T6_MOD_RIFLE_BULLET);
        copy.adopt_states(&host.wire_list());
        assert_eq!(copy.state_of(0), PaneShardCondition::Shattered);
        assert_eq!(copy.state_of(1), PaneShardCondition::Weakened);
        assert_eq!(copy.trace_damage(), vec![(0, PANE_SHATTER_HARM), (1, PANE_CRACK_HARM)]);
        assert!(copy.take_smashes().is_empty());
    }

    #[test]
    fn a_shot_is_stopped_by_a_pane_only_under_the_destroy_damage() {
        assert!(shard_blocks_shots(0));
        assert!(shard_blocks_shots(PANE_SHATTER_HARM - 1));
        assert!(!shard_blocks_shots(PANE_SHATTER_HARM));
    }

    #[test]
    fn trace_damage_adds_up_and_saturates() {
        assert_eq!(pile_harm(10, 5), 15);
        assert_eq!(pile_harm(65_530, 10), u16::MAX);
        assert_eq!(pile_harm(1, u32::MAX), u16::MAX);
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
