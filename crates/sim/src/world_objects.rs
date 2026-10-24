use crate::identities::ScriptModelId;

pub type GlassPieceId = u32;

/// A Black Ops II pane's state (BO2's 2-bit `GlassState`) by the names
/// the callers use: pristine, cracked, shattered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlassPieceState {
    #[default]
    Intact,
    Weakened,
    Shattered,
}

/// The damage a pane's state stands for in `glass_damage_pairs` (a shot's
/// trace reads which panes still stand from it).
pub const GLASS_DAMAGE_TO_WEAKEN: u16 = 25;
pub const GLASS_DAMAGE_TO_DESTROY: u16 = 100;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldObjectSnapshot {
    pub as_of_ms: i32,
    pub map_round_epoch: u32,
    /// bo2mp: every Black Ops II pane's state (and a shattered pane's hit),
    /// in pane order, as BO2's snapshot carries it; empty on other maps.
    pub t6_glass: Vec<glass_t6::PaneUpdate>,

    pub destructible_loop_sounds: Vec<DestructibleLoopSound>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DestructibleLoopSound {
    pub owner: ScriptModelId,
    pub alias_index: u8,
    pub origin: [f32; 3],
    /// The loop's volume and pitch scale in hundredths (`setloopstate`;
    /// 100 = the alias's own level and pitch).
    pub volume: u8,
    pub pitch: u8,
}

impl DestructibleLoopSound {
    /// A scale (0..2.55) as the row's hundredths.
    pub fn hundredths(scale: f32) -> u8 {
        (scale * 100.0).round().clamp(0.0, 255.0) as u8
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldObjectState {
    destructible_loop_sounds: Vec<DestructibleLoopSound>,

    map_round_epoch: u32,

    /// Black Ops II server glass (`GlassesServer`); `t6_ids[i]` is pane
    /// i's glass piece, ascending. Empty on maps without BO2 glass.
    t6_ids: Vec<GlassPieceId>,
    t6: GlassesServer,
    /// BO2 shatters not yet told to the scripts
    /// (`codecallback_glasssmash(pos, dir)`), oldest first.
    glass_smashes: Vec<([f32; 3], [f32; 3])>,
}

/// One Black Ops II glass pane as the map gives it (`Glass::origin`,
/// `GlassDef::maxHealth`).
pub type T6GlassPane = glass_t6::PaneDef;

// BO2's glass rules and means of death live in glass_t6; the old names stay
// for the callers.
pub use glass_t6::server::{
    DAMAGE_MULTIPLIER as T6_GLASS_DAMAGE_MULTIPLIER, MEANS_NAMES as T6_MEANS_NAMES,
    MELEE_DAMAGE as T6_GLASS_MELEE_DAMAGE, MOD_EXPLOSIVE as T6_MOD_EXPLOSIVE,
    MOD_GRENADE_SPLASH as T6_MOD_GRENADE_SPLASH, MOD_IMPACT as T6_MOD_IMPACT,
    MOD_MELEE as T6_MOD_MELEE, MOD_PROJECTILE_SPLASH as T6_MOD_PROJECTILE_SPLASH,
    MOD_RIFLE_BULLET as T6_MOD_RIFLE_BULLET, MOD_UNKNOWN as T6_MOD_UNKNOWN,
    means_from_name as t6_means_from_name, means_name as t6_means_name,
};
use glass_t6::{Cause, GlassesServer, PaneState};

impl WorldObjectState {
    pub fn set_destructible_loop_sounds(&mut self, rows: Vec<DestructibleLoopSound>) {
        self.destructible_loop_sounds = rows;
    }

    /// A pane's state, from the BO2 server itself.
    pub fn glass_piece_state(&self, id: GlassPieceId) -> GlassPieceState {
        self.t6_state(id).unwrap_or_default()
    }

    pub fn glass_damage(&self, id: GlassPieceId) -> u16 {
        glass_damage_for_state(self.glass_piece_state(id))
    }

    pub fn glass_is_solid(&self, id: GlassPieceId) -> bool {
        self.glass_piece_state(id) != GlassPieceState::Shattered
    }

    /// Every hit pane's damage (its state's).
    pub fn glass_damage_pairs(&self) -> Vec<(GlassPieceId, u16)> {
        self.t6_ids
            .iter()
            .zip(&self.t6.panes)
            .filter(|(_, p)| p.state != PaneState::Pristine)
            .map(|(id, _)| (*id, self.glass_damage(*id)))
            .collect()
    }

    pub fn script_destroy_glass(&mut self, id: GlassPieceId, at_time_ms: i32) -> GlassPieceState {
        let hit = self
            .t6_pane(id)
            .and_then(|pane| self.t6.pane(pane))
            .map_or([0.0; 3], |p| p.def.origin);
        self.force_shatter_glass(id, at_time_ms, hit, [0.0, 0.0, 1.0])
    }

    pub fn force_shatter_glass(
        &mut self,
        id: GlassPieceId,
        at_time_ms: i32,
        hit: [f32; 3],
        dir: [f32; 3],
    ) -> GlassPieceState {
        if !self.glass_is_solid(id) {
            return self.glass_piece_state(id);
        }
        if let Some(pane) = self.t6_pane(id) {
            // An impact takes a pane's whole health.
            self.t6.damage(pane, 0, T6_MOD_IMPACT, hit, dir, at_time_ms, Cause::Hit, &mut || 0.0);
            self.t6_sync();
        }
        self.glass_piece_state(id)
    }

    pub fn set_map_round_epoch(&mut self, epoch: u32) {
        self.map_round_epoch = epoch;
    }

    pub fn map_round_epoch(&self) -> u32 {
        self.map_round_epoch
    }

    pub fn clone_glass_panes_from(&mut self, other: &Self) {
        self.t6_ids = other.t6_ids.clone();
        self.t6 = GlassesServer::new(other.t6.panes.iter().map(|p| p.def));
    }

    /// The map's Black Ops II glass: from here on these panes take damage
    /// by BO2's rules (`GlassesServer`), each at its GlassDef's full health.
    pub fn install_t6_glass(&mut self, mut panes: Vec<(GlassPieceId, T6GlassPane)>) {
        panes.sort_by_key(|(id, _)| *id);
        self.t6_ids = panes.iter().map(|(id, _)| *id).collect();
        self.t6 = GlassesServer::new(panes.into_iter().map(|(_, def)| def));
        self.glass_smashes.clear();
    }

    pub fn has_t6_glass(&self) -> bool {
        !self.t6_ids.is_empty()
    }

    pub fn is_t6_glass(&self, id: GlassPieceId) -> bool {
        self.t6_pane(id).is_some()
    }

    /// A BO2 pane's health now (None: not a BO2 pane).
    pub fn t6_glass_health(&self, id: GlassPieceId) -> Option<f32> {
        Some(self.t6.pane(self.t6_pane(id)?)?.health)
    }

    /// Every BO2 pane's health now (a shot's trace asks which still stand).
    pub fn t6_glass_healths(&self) -> Vec<(GlassPieceId, f32)> {
        self.t6_ids.iter().zip(&self.t6.panes).map(|(id, p)| (*id, p.health)).collect()
    }

    fn t6_pane(&self, id: GlassPieceId) -> Option<u32> {
        self.t6_ids.binary_search(&id).ok().map(|i| i as u32)
    }

    /// A BO2 pane's state by the old names (None: not a BO2 pane).
    fn t6_state(&self, id: GlassPieceId) -> Option<GlassPieceState> {
        Some(match self.t6.pane(self.t6_pane(id)?)?.state {
            PaneState::Pristine => GlassPieceState::Intact,
            PaneState::Cracked => GlassPieceState::Weakened,
            PaneState::Shattered => GlassPieceState::Shattered,
        })
    }

    /// Hands the BO2 server's shatters to the scripts. Its states travel
    /// by themselves (the snapshot's `t6_glass`).
    fn t6_sync(&mut self) {
        self.t6.take_changes();
        for smash in self.t6.take_smashes() {
            if t6_glass_log() {
                diag::info!(
                    Sim,
                    "bo2mp t6 glass: pane {} shattered, smash at {:?} dir {:?}",
                    self.t6_ids.get(smash.pane as usize).copied().unwrap_or(u32::MAX),
                    smash.origin,
                    smash.dir,
                );
            }
            self.glass_smashes.push((smash.origin, smash.dir));
        }
    }

    /// The BO2 shatters since the last call, for `codecallback_glasssmash`.
    pub fn take_glass_smashes(&mut self) -> Vec<([f32; 3], [f32; 3])> {
        std::mem::take(&mut self.glass_smashes)
    }

    /// GlassSv_Damage: one hit on a BO2 pane. Grenades, projectiles, melee
    /// and impacts take all its health; other kinds take `damage` once it
    /// is at least 5 (less does nothing). Out of health it shatters (and
    /// the scripts hear `glass_smash`); otherwise its first hit cracks it,
    /// and a cracked pane then bleeds 1-10 health a second until it breaks.
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
        let Some(pane) = self.t6_pane(id) else {
            return self.glass_piece_state(id);
        };
        let before = self.t6.panes[pane as usize].health;
        self.t6.damage(pane, damage, means, hit, dir, at_time_ms, Cause::Hit, next_random);
        self.t6_sync();
        if t6_glass_log() {
            let p = self.t6.panes[pane as usize];
            diag::info!(
                Sim,
                "bo2mp t6 glass: pane {id} hit {damage} means {means} at {at_time_ms} ms: health {before} -> {}, {:?}, bleeds {}/s",
                p.health.max(0.0),
                self.glass_piece_state(id),
                p.cracked_rate,
            );
        }
        self.glass_piece_state(id)
    }

    /// GlassSv_RadiusDamage: every BO2 pane whose origin is inside the
    /// blast (and its cone, when it has one) takes the blast's damage at
    /// that distance, max at the centre down to min at the edge.
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
        self.t6.radius_damage(origin, radius, cone, max_damage, min_damage, means, at_time_ms, next_random);
        self.t6_sync();
    }

    /// A streak's bullet on a pane: BO2's rule for a bullet.
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
        let damage = i32::try_from(damage).unwrap_or(i32::MAX);
        self.apply_t6_glass_damage(id, damage, T6_MOD_RIFLE_BULLET, at_time_ms, hit, dir, next_random)
    }

    /// GlassesServer::Update: each cracked BO2 pane loses its bleed rate
    /// times the frame's time, and breaks by itself once below zero.
    pub fn glass_update(&mut self, at_time_ms: i32) {
        if self.t6_ids.is_empty() {
            return;
        }
        self.t6.update(at_time_ms);
        self.t6_sync();
    }

    pub fn to_snapshot(&self) -> WorldObjectSnapshot {
        WorldObjectSnapshot {
            as_of_ms: 0,
            map_round_epoch: self.map_round_epoch,
            t6_glass: glass_t6::net::full_list(&self.t6.panes),
            destructible_loop_sounds: self.destructible_loop_sounds.clone(),
        }
    }

    pub fn adopt_snapshot(&mut self, snap: &WorldObjectSnapshot) {
        self.map_round_epoch = snap.map_round_epoch;
        self.destructible_loop_sounds = snap.destructible_loop_sounds.clone();
        // The BO2 health stays the server's; a copy only shows the states.
        self.t6.reset();
        self.glass_smashes.clear();
        for u in &snap.t6_glass {
            let Some(p) = self.t6.panes.get_mut(u.pane as usize) else {
                continue;
            };
            p.state = u.state;
            if let Some((pos, dir)) = u.hit {
                p.hit_pos = pos;
                p.hit_dir = dir;
            }
            if u.state == PaneState::Shattered {
                p.health = 0.0;
            }
        }
    }
}

/// Whether a pane with this `glass_damage` still stands.
pub(crate) fn glass_piece_is_solid(damage: u16) -> bool {
    damage < GLASS_DAMAGE_TO_DESTROY
}

/// A shot's own running count of a pane's damage (capped).
pub(crate) fn glass_add_damage(current: u16, added: u32) -> u16 {
    u16::try_from(u32::from(current).saturating_add(added))
        .unwrap_or(u16::MAX)
}

fn glass_damage_for_state(state: GlassPieceState) -> u16 {
    match state {
        GlassPieceState::Intact => 0,
        GlassPieceState::Weakened => GLASS_DAMAGE_TO_WEAKEN,
        GlassPieceState::Shattered => GLASS_DAMAGE_TO_DESTROY,
    }
}

/// Test aid (BO2MP_T6_HITLOG=1): each BO2 pane's hits and breaks in the log.
fn t6_glass_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_T6_HITLOG").is_some())
}

#[cfg(test)]
mod t6_glass_tests {
    use super::*;

    fn raid_pane(origin: [f32; 3]) -> T6GlassPane {
        // Raid's panes: glass_dec_clear_mp, maxHealth 120.
        T6GlassPane { origin, absmin: origin, absmax: origin, max_health: 120.0 }
    }

    fn world(panes: &[[f32; 3]]) -> WorldObjectState {
        let mut w = WorldObjectState::default();
        w.install_t6_glass(
            panes
                .iter()
                .enumerate()
                .map(|(i, o)| (i as GlassPieceId, raid_pane(*o)))
                .collect(),
        );
        w
    }

    fn hit(w: &mut WorldObjectState, id: GlassPieceId, damage: i32, means: u8) -> GlassPieceState {
        w.apply_t6_glass_damage(
            id,
            damage,
            means,
            1000,
            [0.0; 3],
            [1.0, 0.0, 0.0],
            &mut || 0.5,
        )
    }

    #[test]
    fn under_five_does_nothing_and_five_or_more_cracks() {
        let mut w = world(&[[0.0; 3]]);
        assert_eq!(hit(&mut w, 0, 4, T6_MOD_RIFLE_BULLET), GlassPieceState::Intact);
        assert_eq!(w.t6_glass_health(0), Some(120.0));
        assert_eq!(hit(&mut w, 0, 30, T6_MOD_RIFLE_BULLET), GlassPieceState::Weakened);
        assert_eq!(w.t6_glass_health(0), Some(90.0));
        assert!(w.take_glass_smashes().is_empty());
        assert_eq!(hit(&mut w, 0, 90, T6_MOD_RIFLE_BULLET), GlassPieceState::Shattered);
        assert_eq!(w.take_glass_smashes(), vec![([0.0; 3], [1.0, 0.0, 0.0])]);
    }

    #[test]
    fn melee_impact_and_grenades_shatter_at_once() {
        for means in [3, 5, T6_MOD_MELEE, 8, T6_MOD_IMPACT, 17] {
            let mut w = world(&[[0.0; 3]]);
            assert_eq!(hit(&mut w, 0, 1, means), GlassPieceState::Shattered, "means {means}");
        }
        // A grenade's splash is not a grenade: it takes its damage.
        let mut w = world(&[[0.0; 3]]);
        assert_eq!(hit(&mut w, 0, 40, T6_MOD_GRENADE_SPLASH), GlassPieceState::Weakened);
        assert_eq!(w.t6_glass_health(0), Some(80.0));
    }

    #[test]
    fn a_cracked_pane_bleeds_until_it_breaks() {
        let mut w = world(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        // Rate 1 + 9 * 0.5 = 5.5 a second; 90 health left breaks after ~16.4 s.
        hit(&mut w, 0, 30, T6_MOD_RIFLE_BULLET);
        let mut now = 1000;
        w.glass_update(now);
        while now < 1000 + 16_300 {
            now += 50;
            w.glass_update(now);
        }
        assert_eq!(w.glass_piece_state(0), GlassPieceState::Weakened);
        while now < 1000 + 16_500 {
            now += 50;
            w.glass_update(now);
        }
        assert_eq!(w.glass_piece_state(0), GlassPieceState::Shattered);
        assert_eq!(w.take_glass_smashes(), vec![([0.0; 3], [0.0; 3])]);
        // The pane nobody hit stands whole.
        assert_eq!(w.glass_piece_state(1), GlassPieceState::Intact);
        assert_eq!(w.t6_glass_health(1), Some(120.0));
    }

    #[test]
    fn a_blast_falls_off_to_its_edge() {
        let mut w = world(&[[100.0, 0.0, 0.0], [250.0, 0.0, 0.0]]);
        w.apply_t6_glass_radius([0.0; 3], 200.0, None, 100.0, 20.0, T6_MOD_EXPLOSIVE, 1000, &mut || 0.5);
        // Half way out: (1 - 0.5) * (100 - 20) + 20 = 60.
        assert_eq!(w.t6_glass_health(0), Some(60.0));
        assert_eq!(w.glass_piece_state(0), GlassPieceState::Weakened);
        assert_eq!(w.t6_glass_health(1), Some(120.0));
        // Outside a cone nothing is hit.
        let mut w = world(&[[100.0, 0.0, 0.0]]);
        let cone = Some(([0.0, 1.0, 0.0], 0.5));
        w.apply_t6_glass_radius([0.0; 3], 200.0, cone, 100.0, 20.0, T6_MOD_EXPLOSIVE, 1000, &mut || 0.5);
        assert_eq!(w.t6_glass_health(0), Some(120.0));
    }

    #[test]
    fn means_names_match_bo2s_table() {
        assert_eq!(t6_means_from_name("MOD_RIFLE_BULLET"), T6_MOD_RIFLE_BULLET);
        assert_eq!(t6_means_from_name("mod_melee"), T6_MOD_MELEE);
        assert_eq!(t6_means_from_name("MOD_EXPLOSIVE"), T6_MOD_EXPLOSIVE);
        assert_eq!(t6_means_from_name("MOD_GAS"), 20);
        assert_eq!(t6_means_from_name("MOD_NOT_A_KIND"), T6_MOD_UNKNOWN);
    }

    #[test]
    fn a_client_copy_takes_bo2_states_from_the_snapshot() {
        let mut server = world(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        let mut client = world(&[[0.0; 3], [64.0, 0.0, 0.0]]);
        hit(&mut server, 0, 1, T6_MOD_MELEE);
        // A streak's bullet goes through BO2's rule: 30 cracks, never shatters.
        let state = server.apply_glass_hit(1, 30, 1000, [64.0, 0.0, 0.0], [1.0, 0.0, 0.0], &mut || 0.5);
        assert_eq!(state, GlassPieceState::Weakened);
        let snap = server.to_snapshot();
        client.adopt_snapshot(&snap);
        assert!(!client.glass_is_solid(0));
        assert_eq!(client.glass_piece_state(1), GlassPieceState::Weakened);
        assert!(client.glass_is_solid(1));
        assert_eq!(
            client.glass_damage_pairs(),
            vec![(0, GLASS_DAMAGE_TO_DESTROY), (1, GLASS_DAMAGE_TO_WEAKEN)]
        );
    }
}
