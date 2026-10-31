//! What each client keeps per pane and what a state change sets off
//! (`CG_Glass_SetState` and `PlayShatterFX`). The host turns the cues into
//! materials, sounds, effects and shards; this module only decides them.

use std::collections::VecDeque;

use crate::server::PaneState;

/// Edges shorter than this end the shatter FX loop (`shatterFxMinEdgeLength`).
pub const SHATTER_FX_MIN_EDGE: f32 = 20.0;
/// One FX per this much edge, 1 to 3 per edge (`shatterFxMaxDist`).
pub const SHATTER_FX_SPACING: f32 = 55.0;
/// Checked before each edge, so up to 12 can spawn.
pub const SHATTER_FX_LIMIT: usize = 10;
/// Panes cut per frame (`maxGlassShatters` at its default; the dvar runs
/// 1..999). Gamestate shatters are not counted.
pub const MAX_GLASS_SHATTERS: usize = 2;

/// One pane as the client keeps it. `state` is None until the gamestate
/// arrives (BO2's INVALID, never sent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientPane {
    pub state: Option<PaneState>,
    /// Set when a shatter is queued; the intact pane is still drawn until
    /// the shards are cut.
    pub shatter_delay: bool,
}

/// What a state change asks the host to do, in BO2's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    /// Remove this pane's shards.
    RemoveShards { pane: u32 },
    /// Write the pristine or cracked material into the pane's brush.
    Material { pane: u32, cracked: bool },
    /// `crackSound` at the pane origin.
    CrackSound { pane: u32 },
    /// Cut the pane into shards; a gamestate shatter is not counted against
    /// the shatter cap, and its simple-body shards go at once.
    Shatter { pane: u32, impact_point: [f32; 3], hit_dir: [f32; 3], gamestate: bool },
    /// Client script notify `glass_smash(origin, hitDir)`.
    ScriptSmash { pane: u32, hit_dir: [f32; 3] },
    /// `shatterSound`, or `autoShatterSound` for a zero hit direction.
    ShatterSound { pane: u32, auto: bool },
    /// Play the shatter FX along the pane's outline (`shatter_fx_points`).
    ShatterFx { pane: u32, hit_dir: [f32; 3] },
    /// Drop the bullet marks stuck to the pane.
    DetachMarks { pane: u32 },
}

/// All the map's panes on one client (`cgGlasses`).
#[derive(Clone, Debug, Default)]
pub struct GlassesClient {
    pub panes: Vec<ClientPane>,
    cues: Vec<Cue>,
    /// Shatters waiting to be cut, oldest first (BO2's action ring).
    /// TODO(fact): the ring holds 200 and is shared with the actions on
    /// pieces; what BO2 does when it is full is not in the facts, so this
    /// queue has no cap. Also not in the facts: whether a waiting shatter is
    /// dropped when its pane goes back to pristine before it is cut.
    shatters: VecDeque<Cue>,
}

impl GlassesClient {
    pub fn new(pane_count: usize) -> Self {
        Self {
            panes: vec![ClientPane::default(); pane_count],
            cues: Vec::new(),
            shatters: VecDeque::new(),
        }
    }

    pub fn state(&self, pane: u32) -> Option<PaneState> {
        self.panes.get(pane as usize).and_then(|p| p.state)
    }

    /// Applies a state from a snapshot or the gamestate. The same state
    /// again does nothing; a pane that went through cracked between two
    /// snapshots arrives only as shattered and plays the full shatter.
    pub fn set_state(&mut self, pane: u32, to_state: PaneState, impact_point: [f32; 3], hit_dir: [f32; 3], gamestate: bool) {
        let Some(p) = self.panes.get_mut(pane as usize) else { return };
        if p.state == Some(to_state) {
            return;
        }
        p.state = Some(to_state);
        match to_state {
            PaneState::Pristine | PaneState::Cracked => {
                let cracked = to_state == PaneState::Cracked;
                self.cues.push(Cue::RemoveShards { pane });
                self.cues.push(Cue::Material { pane, cracked });
                if cracked && !gamestate {
                    self.cues.push(Cue::CrackSound { pane });
                }
            }
            PaneState::Shattered => {
                p.shatter_delay = true;
                self.shatters.push_back(Cue::Shatter { pane, impact_point, hit_dir, gamestate });
                self.cues.push(Cue::ScriptSmash { pane, hit_dir });
                if !gamestate {
                    self.cues.push(Cue::ShatterSound { pane, auto: hit_dir == [0.0; 3] });
                    self.cues.push(Cue::ShatterFx { pane, hit_dir });
                    self.cues.push(Cue::DetachMarks { pane });
                }
            }
        }
    }

    /// The host cut the queued shatter: stop drawing the intact pane.
    pub fn shatter_started(&mut self, pane: u32) {
        if let Some(p) = self.panes.get_mut(pane as usize) {
            p.shatter_delay = false;
        }
    }

    /// Whether the intact brush is drawn: unless shattered, and while a
    /// queued shatter has not been cut yet.
    pub fn draws_brush(&self, pane: u32) -> bool {
        self.panes
            .get(pane as usize)
            .is_some_and(|p| p.state != Some(PaneState::Shattered) || p.shatter_delay)
    }

    /// Whether the brush blocks: pristine and cracked the same, shattered
    /// never (even while still drawn).
    pub fn collides(&self, pane: u32) -> bool {
        self.panes.get(pane as usize).is_some_and(|p| p.state != Some(PaneState::Shattered))
    }

    /// Map restart: shards go (the host clears them), states stay; the new
    /// gamestate then applies through `set_state`.
    pub fn wipe_all(&mut self) {
        for p in &mut self.panes {
            p.shatter_delay = false;
        }
        self.cues.clear();
        self.shatters.clear();
    }

    /// This frame's cues: the state changes' at once, then the waiting
    /// shatters in order, up to `MAX_GLASS_SHATTERS` counted ones; the rest
    /// wait for later frames with their intact panes still drawn.
    pub fn take_cues(&mut self) -> Vec<Cue> {
        let mut out = std::mem::take(&mut self.cues);
        let mut counted = 0;
        while let Some(&c) = self.shatters.front() {
            let free = matches!(c, Cue::Shatter { gamestate, .. } if gamestate);
            if !free {
                if counted == MAX_GLASS_SHATTERS {
                    break;
                }
                counted += 1;
            }
            out.push(c);
            self.shatters.pop_front();
        }
        out
    }
}

/// One shatter FX: where it plays and the way it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxPoint {
    pub play_at: [f32; 3],
    pub facing: [f32; 3],
}

fn sub(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    [lhs[0] - rhs[0], lhs[1] - rhs[1], lhs[2] - rhs[2]]
}

fn dot(u: [f32; 3], v: [f32; 3]) -> f32 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

/// Where the shatter FX of a pane with this closed outline play. Edges go
/// longest first and the loop ends at the first one under 20; each edge
/// gets round(length / 55) clamped to 1..3 effects at the middles of equal
/// segments, and the limit of 10 is checked before each edge. Each faces
/// along the normal, flipped to the hit direction's side; `zero_dir_flip`
/// picks the side for a zero direction (BO2 rolls it).
pub fn shatter_fx_points(outline: &[[f32; 3]], nrm: [f32; 3], hit_dir: [f32; 3], zero_dir_flip: bool) -> Vec<FxPoint> {
    let flip = if hit_dir == [0.0; 3] { zero_dir_flip } else { dot(nrm, hit_dir) < 0.0 };
    let dir = if flip { nrm.map(|v| -v) } else { nrm };
    let mut edges: Vec<(f32, [f32; 3], [f32; 3])> = (0..outline.len())
        .map(|i| {
            let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
            (dot(sub(b, a), sub(b, a)).sqrt(), a, b)
        })
        .collect();
    edges.sort_by(|x, y| y.0.total_cmp(&x.0));
    let mut sparks = Vec::new();
    for (len, a, b) in edges {
        if len < SHATTER_FX_MIN_EDGE || sparks.len() >= SHATTER_FX_LIMIT {
            break;
        }
        let n = ((len / SHATTER_FX_SPACING).round() as usize).clamp(1, 3);
        for k in 0..n {
            let t = (k as f32 + 0.5) / n as f32;
            let pos = [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t);
            sparks.push(FxPoint { play_at: pos, facing: dir });
        }
    }
    sparks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crack_swaps_material_and_sounds_once() {
        let mut g = GlassesClient::new(2);
        g.set_state(1, PaneState::Cracked, [0.0; 3], [0.0; 3], false);
        g.set_state(1, PaneState::Cracked, [0.0; 3], [0.0; 3], false);
        assert_eq!(
            g.take_cues(),
            vec![
                Cue::RemoveShards { pane: 1 },
                Cue::Material { pane: 1, cracked: true },
                Cue::CrackSound { pane: 1 },
            ]
        );
        assert!(g.collides(1) && g.draws_brush(1));
    }

    #[test]
    fn gamestate_crack_and_shatter_are_silent() {
        let mut g = GlassesClient::new(2);
        g.set_state(0, PaneState::Cracked, [0.0; 3], [0.0; 3], true);
        g.set_state(1, PaneState::Shattered, [1.0; 3], [0.0, 1.0, 0.0], true);
        let cues = g.take_cues();
        assert!(!cues.iter().any(|c| matches!(
            c,
            Cue::CrackSound { .. } | Cue::ShatterSound { .. } | Cue::ShatterFx { .. }
        )));
        assert!(cues.contains(&Cue::ScriptSmash { pane: 1, hit_dir: [0.0, 1.0, 0.0] }));
    }

    #[test]
    fn shatter_stops_collision_at_once_but_draws_until_cut() {
        let mut g = GlassesClient::new(1);
        g.set_state(0, PaneState::Shattered, [0.0; 3], [0.0; 3], false);
        assert!(!g.collides(0));
        assert!(g.draws_brush(0));
        assert!(g.take_cues().contains(&Cue::ShatterSound { pane: 0, auto: true }));
        g.shatter_started(0);
        assert!(!g.draws_brush(0));
        g.wipe_all();
        assert_eq!(g.state(0), Some(PaneState::Shattered));
    }

    #[test]
    fn two_panes_cut_per_frame_and_gamestate_ones_are_free() {
        let mut g = GlassesClient::new(6);
        for pane in 0..3 {
            g.set_state(pane, PaneState::Shattered, [0.0; 3], [0.0; 3], true);
        }
        let shatters = |cues: &[Cue]| -> Vec<u32> {
            cues.iter()
                .filter_map(|c| match c {
                    Cue::Shatter { pane, .. } => Some(*pane),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(shatters(&g.take_cues()), vec![0, 1, 2]);
        for pane in 3..6 {
            g.set_state(pane, PaneState::Shattered, [0.0; 3], [1.0, 0.0, 0.0], false);
        }
        let first = g.take_cues();
        assert_eq!(shatters(&first), vec![3, 4]);
        // The waiting pane's sound and FX still play at once.
        assert!(first.contains(&Cue::ShatterSound { pane: 5, auto: false }));
        g.shatter_started(3);
        g.shatter_started(4);
        assert!(g.draws_brush(5) && !g.collides(5));
        assert_eq!(shatters(&g.take_cues()), vec![5]);
        assert!(g.take_cues().is_empty());
    }

    #[test]
    fn fx_follow_the_longest_edges() {
        // 120 x 30 pane: two 120 edges (2 each), two 30 edges (1 each).
        let o = [[0.0, 0.0, 0.0], [120.0, 0.0, 0.0], [120.0, 0.0, 30.0], [0.0, 0.0, 30.0]];
        let pts = shatter_fx_points(&o, [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], false);
        assert_eq!(pts.len(), 6);
        assert_eq!(pts[0].play_at, [30.0, 0.0, 0.0]);
        assert!(pts.iter().all(|p| p.facing == [0.0, -1.0, 0.0]));
        // Short edges stop the loop.
        let tiny = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 0.0, 10.0], [0.0, 0.0, 10.0]];
        assert_eq!(shatter_fx_points(&tiny, [0.0, 1.0, 0.0], [0.0; 3], false).len(), 4);
    }

    #[test]
    fn fx_limit_is_checked_before_each_edge() {
        // Eight 200-long edges (3 each): 3, 6, 9, 12, then the limit.
        let o: Vec<[f32; 3]> = (0..8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                [a.cos() * 261.3, 0.0, a.sin() * 261.3]
            })
            .collect();
        assert_eq!(shatter_fx_points(&o, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], false).len(), 12);
    }
}
