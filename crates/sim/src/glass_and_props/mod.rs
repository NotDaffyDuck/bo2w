//! Set pieces the server tracks apart from the entity table: the map's glass, the looping sounds of destructible
//! objects, and the counter that says which round of a map a client's glass belongs to.

mod glass;

pub use self::glass::{
    Blast, Blow, PANE_SHATTER_HARM, PANE_CRACK_HARM, PaneShardCondition, Strike,
    T6_GLASS_DAMAGE_MULTIPLIER, T6_GLASS_MELEE_DAMAGE, T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH, T6_MOD_IMPACT,
    T6_MOD_MELEE, T6_MOD_PROJECTILE_SPLASH, T6_MOD_RIFLE_BULLET, T6_MOD_UNKNOWN, t6_means_from_name,
    t6_means_name,
};
pub(crate) use self::glass::{pile_harm, shard_blocks_shots};

use crate::match_ids::ScriptedModel;
use glass::GlassField;

/// A sound a destructible object keeps looping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DestructibleLoopSound {
    pub owner: ScriptedModel,
    pub alias_index: u8,
    pub origin: [f32; 3],
    /// Volume and pitch in hundredths of the alias's own values; 100 plays it as authored.
    pub volume: u8,
    pub pitch: u8,
}

impl DestructibleLoopSound {
    /// A scale (0 to 2.55) in hundredths, held to what a byte carries.
    pub fn hundredths(scale: f32) -> u8 {
        (scale * 100.0).round().clamp(0.0, f32::from(u8::MAX)) as u8
    }
}

/// What the host tells a client about these set pieces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropsSnapshot {
    pub as_of_ms: i32,
    pub map_round_epoch: u32,
    /// The state of every BO2 pane in pane order, with the hit that broke each shattered one. Empty without glass.
    pub t6_glass: Vec<glass_t6::PaneUpdate>,
    pub destructible_loop_sounds: Vec<DestructibleLoopSound>,
}

/// All the set pieces in the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapPropState {
    loop_sounds: Vec<DestructibleLoopSound>,
    map_round_epoch: u32,
    glass: GlassField,
}

impl MapPropState {
    pub fn set_destructible_loop_sounds(&mut self, sounds: Vec<DestructibleLoopSound>) {
        self.loop_sounds = sounds;
    }

    pub fn set_map_round_epoch(&mut self, round: u32) {
        self.map_round_epoch = round;
    }

    pub fn map_round_epoch(&self) -> u32 {
        self.map_round_epoch
    }

    /// Gives the map its BO2 glass; every pane starts at full health and takes damage by BO2's rules.
    pub fn install_t6_glass(&mut self, panes: Vec<(u32, glass_t6::PaneDef)>) {
        self.glass.install(panes);
    }

    /// Takes the panes of another world (same map, next round), undamaged.
    pub fn fresh_panes_like(&mut self, donor: &Self) {
        self.glass.reset_to_layout_of(&donor.glass);
    }

    pub fn has_t6_glass(&self) -> bool {
        !self.glass.has_no_panes()
    }

    pub fn is_t6_glass(&self, id: u32) -> bool {
        self.glass.is_pane(id)
    }

    /// The state of a pane; a piece that is no pane reads as intact.
    pub fn pane_condition(&self, piece: u32) -> PaneShardCondition {
        self.glass.state_of(piece)
    }

    /// Whether the piece still stops shots, as every pane does until it shatters.
    pub fn pane_blocks_shots(&self, piece: u32) -> bool {
        self.glass.state_of(piece) != PaneShardCondition::Shattered
    }

    /// Each damaged pane with the trace damage its state stands for, in piece order: where a shot's trace starts.
    pub fn pane_harm_entries(&self) -> Vec<(u32, u16)> {
        self.glass.trace_damage()
    }

    /// The health of every pane in piece order.
    pub fn t6_glass_healths(&self) -> Vec<(u32, f32)> {
        self.glass.healths()
    }

    /// Hits a pane. One that runs out of health shatters and the scripts hear of it; any other first hit cracks it,
    /// and a cracked pane loses 1 to 10 health a second until it breaks.
    pub fn strike_glass(&mut self, blow: Blow, roll: &mut dyn FnMut() -> f32) -> PaneShardCondition {
        self.glass.strike(blow, roll)
    }

    /// An explosion over the glass: each pane in reach takes the damage its distance gives.
    pub fn blast_glass(&mut self, blast: &Blast, at_ms: i32, roll: &mut dyn FnMut() -> f32) {
        self.glass.blast(blast, at_ms, roll);
    }

    /// Breaks a pane now, like a hit that takes all of its health; a broken pane stays broken.
    pub fn shatter_glass(&mut self, id: u32, at_ms: i32, strike: Strike) -> PaneShardCondition {
        self.glass.shatter(id, at_ms, strike)
    }

    /// A script's `destroy` of a pane: it breaks where it stands and its pieces go upward.
    pub fn destroy_glass(&mut self, id: u32, at_ms: i32) -> PaneShardCondition {
        self.glass.destroy(id, at_ms)
    }

    /// Lets cracked panes bleed up to `at_ms`.
    pub fn bleed_glass(&mut self, upto: i32) {
        self.glass.advance(upto);
    }

    /// The shatters since the last call, for `codecallback_glasssmash`.
    pub fn take_glass_smashes(&mut self) -> Vec<([f32; 3], [f32; 3])> {
        self.glass.take_smashes()
    }

    pub fn export_snapshot(&self) -> PropsSnapshot {
        let snapshot = PropsSnapshot {
            map_round_epoch: self.map_round_epoch,
            t6_glass: self.glass.wire_list(),
            destructible_loop_sounds: self.loop_sounds.clone(),
            as_of_ms: 0,
        };
        snapshot
    }

    pub fn load_props(&mut self, sent: &PropsSnapshot) {
        self.map_round_epoch = sent.map_round_epoch;
        self.loop_sounds.clone_from(&sent.destructible_loop_sounds);
        self.glass.adopt_states(&sent.t6_glass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane_at(origin: [f32; 3]) -> glass_t6::PaneDef {
        glass_t6::PaneDef { origin, absmin: origin, absmax: origin, hit_point_ceiling: 120.0 }
    }

    fn two_panes() -> MapPropState {
        let mut world = MapPropState::default();
        world.install_t6_glass(vec![(0, pane_at([0.0; 3])), (1, pane_at([64.0, 0.0, 0.0]))]);
        world
    }

    #[test]
    fn solid_means_not_shattered() {
        let mut world = two_panes();
        let strike = Strike { at: [0.0; 3], way: [1.0, 0.0, 0.0] };
        world.strike_glass(Blow::bullet(0, 30, 1000, strike), &mut || 0.5);
        assert!(world.pane_blocks_shots(0));
        world.shatter_glass(0, 1100, strike);
        assert!(!world.pane_blocks_shots(0));
        assert!(world.pane_blocks_shots(1));
        assert!(world.pane_blocks_shots(99));
    }

    #[test]
    fn a_client_world_takes_over_the_host_snapshot() {
        let mut host = two_panes();
        let mut client = two_panes();
        let strike = Strike { at: [64.0, 0.0, 0.0], way: [1.0, 0.0, 0.0] };
        host.shatter_glass(0, 1000, strike);
        host.strike_glass(Blow::bullet(1, 30, 1000, strike), &mut || 0.5);
        client.load_props(&host.export_snapshot());
        assert!(!client.pane_blocks_shots(0));
        assert_eq!(client.pane_condition(1), PaneShardCondition::Weakened);
        let expected = vec![(0, PANE_SHATTER_HARM), (1, PANE_CRACK_HARM)];
        assert_eq!(client.pane_harm_entries(), expected);
        assert!(client.take_glass_smashes().is_empty());
    }

    #[test]
    fn the_snapshot_carries_the_round_and_the_loop_sounds() {
        let sound = DestructibleLoopSound {
            owner: ScriptedModel::decode_net(4),
            alias_index: 2,
            origin: [1.0, 2.0, 3.0],
            volume: 100,
            pitch: 50,
        };
        let mut host = MapPropState::default();
        host.set_map_round_epoch(3);
        host.set_destructible_loop_sounds(vec![sound]);
        let snapshot = host.export_snapshot();
        assert_eq!((snapshot.as_of_ms, snapshot.map_round_epoch), (0, 3));
        assert_eq!(snapshot.destructible_loop_sounds, vec![sound]);
        let mut client = MapPropState::default();
        client.load_props(&snapshot);
        assert_eq!(client.map_round_epoch(), 3);
        assert_eq!(client.export_snapshot(), snapshot);
    }

    #[test]
    fn loop_scales_turn_into_byte_sized_hundredths() {
        let cases = [(0.0, 0), (0.5, 50), (1.0, 100), (-3.0, 0), (9.0, 255)];
        for (scale, expected) in cases {
            assert_eq!(DestructibleLoopSound::hundredths(scale), expected, "scale {scale}");
        }
    }
}

/// Older spellings other crates still call; the rename sweep moves them over.
pub use self::PropsSnapshot as WorldObjectSnapshot;
