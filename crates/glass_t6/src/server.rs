//! Black Ops II's server glass (`GlassesServer`, `GlassSv_Damage`,
//! `GlassSv_RadiusDamage`): each pane's health and state. The server only
//! decides WHEN a pane cracks or shatters; the client decides how it looks.
//!
//! A pane starts pristine. A hit of at least 5 (times the damage
//! multiplier) takes that much health; grenades, launched rounds, melee,
//! bayonets, impacts and fire take all of it. A pane left with health is
//! cracked, and a cracked pane bleeds 1-10 health a second until it breaks
//! by itself. Out of health it shatters, keeping the hit's position and
//! direction for the client's falling pieces.

/// `glassDamageMultiplier` default.
pub const DAMAGE_MULTIPLIER: f32 = 1.0;
/// `glassCrackedDamageRateRange` default (health a second).
pub const CRACKED_RATE_RANGE: [f32; 2] = [1.0, 10.0];
/// Hits below this many times the multiplier do nothing (unless instant).
pub const MIN_DAMAGE: f32 = 5.0;
/// `GlassSv_RadiusDamage` looks at no more than this many panes.
pub const RADIUS_DAMAGE_MAX_PANES: usize = 100;

/// BO2's `meansOfDeath_t` numbers glass cares about.
pub const MOD_UNKNOWN: u8 = 0;
pub const MOD_PISTOL_BULLET: u8 = 1;
pub const MOD_RIFLE_BULLET: u8 = 2;
pub const MOD_GRENADE: u8 = 3;
pub const MOD_GRENADE_SPLASH: u8 = 4;
pub const MOD_PROJECTILE: u8 = 5;
pub const MOD_PROJECTILE_SPLASH: u8 = 6;
pub const MOD_MELEE: u8 = 7;
pub const MOD_BAYONET: u8 = 8;
pub const MOD_EXPLOSIVE: u8 = 15;
pub const MOD_IMPACT: u8 = 16;
pub const MOD_BURNED: u8 = 17;
/// BO2 melee's damage to glass (`Weapon_Melee` -> `GlassSv_Damage`).
pub const MELEE_DAMAGE: i32 = 5;

/// BO2's means-of-death names (`modNames`), by number.
pub const MEANS_NAMES: [&str; 21] = [
    "MOD_UNKNOWN",
    "MOD_PISTOL_BULLET",
    "MOD_RIFLE_BULLET",
    "MOD_GRENADE",
    "MOD_GRENADE_SPLASH",
    "MOD_PROJECTILE",
    "MOD_PROJECTILE_SPLASH",
    "MOD_MELEE",
    "MOD_BAYONET",
    "MOD_HEAD_SHOT",
    "MOD_CRUSH",
    "MOD_TELEFRAG",
    "MOD_FALLING",
    "MOD_SUICIDE",
    "MOD_TRIGGER_HURT",
    "MOD_EXPLOSIVE",
    "MOD_IMPACT",
    "MOD_BURNED",
    "MOD_HIT_BY_OBJECT",
    "MOD_DROWN",
    "MOD_GAS",
];

/// `GlassSv_Damage`'s rule for a hit that does not shatter a pane at once:
/// the health it leaves. Under `MIN_DAMAGE` times the multiplier a hit
/// does nothing.
#[must_use]
pub fn health_after_hit(health: f32, dmg_pts: i32, mult: f32) -> f32 {
    if (dmg_pts as f32) < MIN_DAMAGE * mult {
        health
    } else {
        health - ((dmg_pts as f32) * mult) as i32 as f32
    }
}

/// These kinds take a pane's whole health in one hit, whatever the damage
/// (grenade, launched round, melee, bayonet, impact, burned).
pub fn means_shatters(means: u8) -> bool {
    matches!(
        means,
        MOD_GRENADE | MOD_PROJECTILE | MOD_MELEE | MOD_BAYONET | MOD_IMPACT | MOD_BURNED
    )
}

/// A script's means-of-death name as BO2's number; BO2 calls an unknown
/// name a script error and uses MOD_UNKNOWN.
pub fn means_from_name(name: &str) -> u8 {
    MEANS_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
        .map_or(MOD_UNKNOWN, |i| i as u8)
}

/// BO2's name for means of death `m` (MOD_UNKNOWN past the end).
pub fn means_name(m: u8) -> &'static str {
    MEANS_NAMES.get(usize::from(m)).copied().unwrap_or("MOD_UNKNOWN")
}

/// A pane's state, the low bits of `GlassServer::state`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PaneState {
    #[default]
    Pristine = 0,
    Cracked = 1,
    Shattered = 2,
}

impl PaneState {
    pub fn from_bits(bits: u8) -> Self {
        match bits & 3 {
            1 => Self::Cracked,
            2 => Self::Shattered,
            _ => Self::Pristine,
        }
    }
}

/// One pane as the map gives it: `Glass::origin`, its world box
/// (`Glass::absmin`, `absmax`) and its GlassDef's `maxHealth`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneDef {
    pub origin: [f32; 3],
    /// What an explosion's area search tests.
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
    pub hit_point_ceiling: f32,
}

/// One pane as the server keeps it (`GlassServer`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ServerPane {
    pub def: PaneDef,
    pub health: f32,
    pub state: PaneState,
    /// Health a second a cracked pane loses (`crackedDamageRate`).
    pub cracked_rate: f32,
    /// When the state last changed (`stateChangeTime`, ms).
    pub changed_ms: i32,
    /// The shattering hit; both zero for a pane that bled out, which the
    /// client takes as "fall any which way".
    pub impact_point: [f32; 3],
    pub hit_dir: [f32; 3],
}

/// What a shatter tells the scripts (`Scr_GlassSmash` ->
/// `codecallback_glasssmash(origin, dir)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smash {
    pub pane: u32,
    pub origin: [f32; 3],
    pub facing: [f32; 3],
}

/// What caused a change, for logs and the client's sounds only; BO2 keeps
/// no cause.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cause {
    #[default]
    Hit,
    Blast,
    BledOut,
}

/// A pane that changed state this frame, oldest first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneChange {
    pub pane: u32,
    pub state: PaneState,
    pub cause: Cause,
    pub at_ms: i32,
}

/// All the map's panes (`svGlasses`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlassesServer {
    pub panes: Vec<ServerPane>,
    pub damage_multiplier: f32,
    pub cracked_rate_range: [f32; 2],
    last_update_ms: Option<i32>,
    smashes: Vec<Smash>,
    changes: Vec<PaneChange>,
}

impl GlassesServer {
    /// A fresh map: every pane pristine at full health.
    pub fn new(defs: impl IntoIterator<Item = PaneDef>) -> Self {
        Self {
            panes: defs
                .into_iter()
                .map(|def| ServerPane { def, health: def.hit_point_ceiling, ..ServerPane::default() })
                .collect(),
            damage_multiplier: DAMAGE_MULTIPLIER,
            cracked_rate_range: CRACKED_RATE_RANGE,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn pane(&self, pane: u32) -> Option<&ServerPane> {
        self.panes.get(pane as usize)
    }

    pub fn state(&self, pane: u32) -> PaneState {
        self.pane(pane).map_or(PaneState::Pristine, |p| p.state)
    }

    /// Every pane back to pristine and full health (a new round).
    pub fn reset(&mut self) {
        for p in &mut self.panes {
            *p = ServerPane { def: p.def, health: p.def.hit_point_ceiling, ..ServerPane::default() };
        }
        self.last_update_ms = None;
        self.smashes.clear();
        self.changes.clear();
    }

    /// `GlassSv_Damage`: one hit on a pane. `rand01` gives uniform 0..1
    /// (`G_flrand`), used once when the pane first cracks.
    #[allow(clippy::too_many_arguments)]
    pub fn damage(
        &mut self,
        pane: u32,
        hit_pts: i32,
        means: u8,
        impact_point: [f32; 3],
        hit_dir: [f32; 3],
        clock_millis: i32,
        cause: Cause,
        rand01: &mut impl FnMut() -> f32,
    ) -> PaneState {
        let mult = self.damage_multiplier;
        let [lo, hi] = self.cracked_rate_range;
        let Some(p) = self.panes.get_mut(pane as usize) else {
            return PaneState::Pristine;
        };
        if p.health <= 0.0 {
            return p.state;
        }
        if means_shatters(means) {
            p.health = 0.0;
        } else if (hit_pts as f32) < MIN_DAMAGE * mult {
            return p.state;
        } else {
            p.health = health_after_hit(p.health, hit_pts, mult);
        }
        if p.health <= 0.0 {
            if p.state != PaneState::Shattered {
                p.changed_ms = clock_millis;
                p.impact_point = impact_point;
                p.hit_dir = hit_dir;
                p.state = PaneState::Shattered;
                self.smashes.push(Smash { pane, origin: p.def.origin, facing: hit_dir });
                self.changes.push(PaneChange { pane, state: p.state, cause, at_ms: clock_millis });
            }
        } else if p.state != PaneState::Cracked {
            p.changed_ms = clock_millis;
            p.cracked_rate = lo + (hi - lo) * rand01();
            p.state = PaneState::Cracked;
            self.changes.push(PaneChange { pane, state: p.state, cause, at_ms: clock_millis });
        }
        p.state
    }

    /// `GlassSv_RadiusDamage`: of the first 100 standing panes whose box
    /// touches the blast's box (the centre +- radius x 1.41421), each whose
    /// origin is inside the blast (and its cone, when it has one) takes the
    /// blast's damage at that distance, `max_damage` at the centre down to
    /// `min_damage` at the edge. The push it gives the falling pieces grows
    /// with the damage.
    #[allow(clippy::too_many_arguments)]
    pub fn radius_damage(
        &mut self,
        blast_at: [f32; 3],
        blast_r: f32,
        cone: Option<([f32; 3], f32)>,
        max_damage: f32,
        min_damage: f32,
        means: u8,
        clock_millis: i32,
        rand01: &mut impl FnMut() -> f32,
    ) {
        let reach = blast_r * std::f32::consts::SQRT_2;
        let mins = [blast_at[0] - reach, blast_at[1] - reach, blast_at[2] - reach];
        let maxs = [blast_at[0] + reach, blast_at[1] + reach, blast_at[2] + reach];
        for pane in self.area_panes(mins, maxs, RADIUS_DAMAGE_MAX_PANES) {
            let def = self.panes[pane].def;
            let delta = sub(def.origin, blast_at);
            let d2 = dot(delta, delta);
            if !(d2 < blast_r * blast_r) {
                continue;
            }
            let d = d2.sqrt();
            const STRAIGHT_UP: [f32; 3] = [0.0, 0.0, 1.0];
            let unit = if d < 0.01 { STRAIGHT_UP } else { scale(delta, 1.0 / d) };
            if let Some((cone_dir, cone_cos)) = cone {
                if d > 0.01 && !(cone_cos <= dot(unit, cone_dir)) {
                    continue;
                }
            }
            let dmg = (1.0 - d / blast_r) * (max_damage - min_damage) + min_damage;
            let push = dmg / def.hit_point_ceiling + 1.0;
            self.damage(
                pane as u32,
                dmg as i32,
                means,
                blast_at,
                scale(unit, push),
                clock_millis,
                Cause::Blast,
                rand01,
            );
        }
    }

    /// `GlassSv_AreaGlasses`: up to `max` standing panes whose box touches
    /// `mins`..`maxs`, in pane order.
    pub fn area_panes(&self, lo_corner: [f32; 3], hi_corner: [f32; 3], max: usize) -> Vec<usize> {
        self.panes
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.state != PaneState::Shattered
                    && (0..3).all(|i| p.def.absmin[i] <= hi_corner[i] && lo_corner[i] <= p.def.absmax[i])
            })
            .map(|(i, _)| i)
            .take(max)
            .collect()
    }

    /// `GlassesServer::Update`: each cracked pane loses its bleed rate
    /// times the frame's time, and breaks by itself once below zero (no
    /// hit, so its pieces fall any which way).
    pub fn update(&mut self, clock_millis: i32) {
        let frame_ms = self.last_update_ms.map_or(0, |last| clock_millis.saturating_sub(last)).max(0);
        self.last_update_ms = Some(clock_millis);
        if frame_ms == 0 {
            return;
        }
        let mult = self.damage_multiplier;
        for (i, p) in self.panes.iter_mut().enumerate() {
            if p.state != PaneState::Cracked {
                continue;
            }
            p.health -= p.cracked_rate * mult * frame_ms as f32 * 0.001;
            if p.health < 0.0 {
                p.changed_ms = clock_millis;
                p.impact_point = [0.0; 3];
                p.hit_dir = [0.0; 3];
                p.state = PaneState::Shattered;
                let pane = i as u32;
                self.smashes.push(Smash { pane, origin: p.def.origin, facing: [0.0; 3] });
                self.changes.push(PaneChange {
                    pane,
                    state: p.state,
                    cause: Cause::BledOut,
                    at_ms: clock_millis,
                });
            }
        }
    }

    /// The shatters since the last call, for `codecallback_glasssmash`.
    pub fn take_smashes(&mut self) -> Vec<Smash> {
        std::mem::take(&mut self.smashes)
    }

    /// The state changes since the last call, for the network and logs.
    pub fn take_changes(&mut self) -> Vec<PaneChange> {
        std::mem::take(&mut self.changes)
    }
}

fn sub(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    [lhs[0] - rhs[0], lhs[1] - rhs[1], lhs[2] - rhs[2]]
}

fn dot(u: [f32; 3], v: [f32; 3]) -> f32 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

fn scale(vec: [f32; 3], k: f32) -> [f32; 3] {
    [vec[0] * k, vec[1] * k, vec[2] * k]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane_at(origin: [f32; 3], hit_point_ceiling: f32) -> PaneDef {
        let box_at = |d: f32| [origin[0] + d, origin[1] + d, origin[2] + d];
        PaneDef { origin, absmin: box_at(-8.0), absmax: box_at(8.0), hit_point_ceiling }
    }

    fn one_pane(health: f32) -> GlassesServer {
        GlassesServer::new([pane_at([0.0; 3], health)])
    }

    fn hit(g: &mut GlassesServer, dmg: i32, means: u8) -> PaneState {
        g.damage(0, dmg, means, [1.0, 2.0, 3.0], [0.0, 1.0, 0.0], 500, Cause::Hit, &mut || 0.5)
    }

    /// One blast on a lone 120-health pane at `at`; its push, if it was hit.
    fn blast(at: [f32; 3], burst_span: f32, cone: Option<([f32; 3], f32)>, max_damage: f32, min_damage: f32) -> Option<[f32; 3]> {
        let mut g = GlassesServer::new([pane_at(at, 120.0)]);
        g.radius_damage([0.0; 3], burst_span, cone, max_damage, min_damage, MOD_GRENADE_SPLASH, 0, &mut || 0.5);
        (g.state(0) != PaneState::Pristine).then_some(g.panes[0].hit_dir)
    }

    #[test]
    fn small_hits_do_nothing() {
        let mut g = one_pane(100.0);
        let s = g.damage(0, 4, MOD_RIFLE_BULLET, [0.0; 3], [1.0, 0.0, 0.0], 0, Cause::Hit, &mut || 0.5);
        assert_eq!(s, PaneState::Pristine);
        assert_eq!(g.panes[0].health, 100.0);
    }

    #[test]
    fn first_hit_cracks_and_rolls_the_bleed_once() {
        let mut g = one_pane(100.0);
        g.damage(0, 30, MOD_RIFLE_BULLET, [0.0; 3], [1.0, 0.0, 0.0], 0, Cause::Hit, &mut || 0.5);
        assert_eq!(g.state(0), PaneState::Cracked);
        assert_eq!(g.panes[0].cracked_rate, 5.5);
        g.damage(0, 30, MOD_RIFLE_BULLET, [0.0; 3], [1.0, 0.0, 0.0], 0, Cause::Hit, &mut || 1.0);
        assert_eq!(g.panes[0].cracked_rate, 5.5);
        assert_eq!(g.panes[0].health, 40.0);
        assert_eq!(g.take_changes().len(), 1);
    }

    #[test]
    fn instant_kinds_shatter_and_tell_the_scripts() {
        for means in [MOD_GRENADE, MOD_PROJECTILE, MOD_MELEE, MOD_BAYONET, MOD_IMPACT, MOD_BURNED] {
            let mut g = one_pane(1000.0);
            let s = g.damage(0, 1, means, [1.0; 3], [0.0, 1.0, 0.0], 7, Cause::Hit, &mut || 0.0);
            assert_eq!(s, PaneState::Shattered, "{}", means_name(means));
            assert_eq!(g.take_smashes(), vec![Smash { pane: 0, origin: [0.0; 3], facing: [0.0, 1.0, 0.0] }]);
            assert_eq!(g.panes[0].impact_point, [1.0; 3]);
        }
    }

    #[test]
    fn cracked_pane_bleeds_out_with_no_hit() {
        let mut g = one_pane(10.0);
        g.damage(0, 6, MOD_PISTOL_BULLET, [1.0; 3], [1.0, 0.0, 0.0], 0, Cause::Hit, &mut || 1.0);
        g.update(0);
        g.update(300);
        assert_eq!(g.state(0), PaneState::Cracked);
        g.update(500);
        assert_eq!(g.state(0), PaneState::Shattered);
        assert_eq!(g.panes[0].hit_dir, [0.0; 3]);
        let changes = g.take_changes();
        assert_eq!(changes.last().unwrap().cause, Cause::BledOut);
    }

    #[test]
    fn blast_falls_off_and_pushes_harder_when_stronger() {
        let mut g = GlassesServer::new([pane_at([10.0, 0.0, 0.0], 100.0), pane_at([300.0, 0.0, 0.0], 100.0)]);
        g.radius_damage([0.0; 3], 200.0, None, 200.0, 20.0, MOD_GRENADE_SPLASH, 0, &mut || 0.0);
        assert_eq!(g.state(0), PaneState::Shattered);
        assert_eq!(g.state(1), PaneState::Pristine);
        assert!(g.panes[0].hit_dir[0] > 2.0);
    }

    #[test]
    fn a_bullet_under_five_does_nothing() {
        let mut g = one_pane(120.0);
        assert_eq!(hit(&mut g, 4, MOD_RIFLE_BULLET), PaneState::Pristine);
        assert_eq!(g, one_pane(120.0));
    }

    #[test]
    fn the_first_bullet_of_five_or_more_cracks_the_pane() {
        let mut g = one_pane(120.0);
        assert_eq!(hit(&mut g, 30, MOD_RIFLE_BULLET), PaneState::Cracked);
        let p = g.panes[0];
        assert_eq!((p.health, p.cracked_rate, p.changed_ms), (90.0, 5.5, 500));
        assert_eq!(hit(&mut g, 30, MOD_RIFLE_BULLET), PaneState::Cracked);
        assert_eq!(g.panes[0].health, 60.0);
        assert_eq!(g.take_changes().len(), 1);
    }

    #[test]
    fn melee_grenades_and_impacts_shatter_at_once() {
        for means in [MOD_GRENADE, MOD_PROJECTILE, MOD_MELEE, MOD_BAYONET, MOD_IMPACT, MOD_BURNED] {
            let mut g = one_pane(120.0);
            assert_eq!(hit(&mut g, 1, means), PaneState::Shattered, "{}", means_name(means));
            assert_eq!((g.panes[0].impact_point, g.panes[0].hit_dir), ([1.0, 2.0, 3.0], [0.0, 1.0, 0.0]));
            assert_eq!(g.take_smashes().len(), 1);
            hit(&mut g, 1000, means);
            assert!(g.take_smashes().is_empty() && g.take_changes().len() == 1);
        }
    }

    #[test]
    fn enough_bullet_damage_shatters_without_cracking_first() {
        let mut g = one_pane(120.0);
        assert_eq!(hit(&mut g, 120, MOD_RIFLE_BULLET), PaneState::Shattered);
        assert_eq!(g.take_changes().len(), 1);
    }

    #[test]
    fn a_cracked_pane_bleeds_until_below_zero() {
        let mut g = one_pane(120.0);
        hit(&mut g, 115, MOD_RIFLE_BULLET);
        assert_eq!(g.panes[0].health, 5.0);
        // 5.5 a second: 909 ms leaves it just above zero, the next frame breaks it.
        g.update(500);
        g.update(1409);
        assert_eq!(g.state(0), PaneState::Cracked);
        g.update(1459);
        let p = g.panes[0];
        assert_eq!((p.state, p.changed_ms, p.hit_dir), (PaneState::Shattered, 1459, [0.0; 3]));
        let mut fresh = one_pane(120.0);
        fresh.update(0);
        fresh.update(1000);
        assert_eq!(fresh.panes, one_pane(120.0).panes);
    }

    #[test]
    fn radius_damage_falls_from_max_to_min_across_the_radius() {
        assert_eq!(blast([50.0, 0.0, 0.0], 100.0, None, 200.0, 100.0), Some([2.25, 0.0, 0.0]));
        assert_eq!(blast([100.0, 0.0, 0.0], 100.0, None, 200.0, 100.0), None);
        let cone = Some(([0.0, 1.0, 0.0], 0.5));
        assert_eq!(blast([50.0, 0.0, 0.0], 100.0, cone, 200.0, 100.0), None);
        assert_eq!(blast([0.0; 3], 100.0, cone, 240.0, 0.0), Some([0.0, 0.0, 3.0]));
    }

    /// 100 panes whose boxes touch the blast's box but stand outside the
    /// blast, then one at its centre.
    fn crowded() -> GlassesServer {
        let mut defs: Vec<PaneDef> = (0..100).map(|_| pane_at([130.0, 0.0, 0.0], 120.0)).collect();
        defs.push(pane_at([0.0; 3], 120.0));
        GlassesServer::new(defs)
    }

    #[test]
    fn a_blast_looks_at_only_the_first_100_panes_its_box_touches() {
        let mut g = crowded();
        assert_eq!(g.area_panes([-141.42136; 3], [141.42136; 3], 1000).len(), 101);
        g.radius_damage([0.0; 3], 100.0, None, 200.0, 100.0, MOD_GRENADE_SPLASH, 0, &mut || 0.5);
        assert!(g.panes.iter().all(|p| p.state == PaneState::Pristine));
    }

    #[test]
    fn shattered_panes_do_not_count_toward_the_100() {
        let mut g = crowded();
        for p in 0..100 {
            g.damage(p, 0, MOD_IMPACT, [0.0; 3], [0.0; 3], 0, Cause::Hit, &mut || 0.5);
        }
        g.radius_damage([0.0; 3], 100.0, None, 200.0, 100.0, MOD_GRENADE_SPLASH, 0, &mut || 0.5);
        assert_eq!(g.state(100), PaneState::Shattered);
    }

    #[test]
    fn a_hit_takes_its_damage_cut_to_whole_points() {
        assert_eq!(health_after_hit(120.0, 4, 1.0), 120.0);
        assert_eq!(health_after_hit(120.0, 30, 1.0), 90.0);
        assert_eq!(health_after_hit(120.0, 7, 0.5), 117.0);
        assert_eq!(health_after_hit(120.0, 9, 2.0), 120.0);
    }

    #[test]
    fn means_names_round_trip() {
        assert_eq!(means_from_name("mod_melee"), MOD_MELEE);
        assert_eq!(means_name(MOD_BURNED), "MOD_BURNED");
        assert_eq!(means_from_name("nonsense"), MOD_UNKNOWN);
    }
}
