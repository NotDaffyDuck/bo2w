//! How one hit travels through a prop's tree of pieces (BO2's `DamagePiece`).

use super::{Break, Outcome, State};
use crate::world_objects::{T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH, T6_MOD_MELEE, T6_MOD_PROJECTILE_SPLASH};
use xmodel_runtime::T5DestructiblePiece;

// Bits of a stage's flags.
/// The stage holds against everything but a blast.
const HOLDS: u32 = 1;
/// A hit that lands in this stage also hurts the parent piece.
const HURTS_PARENT: u32 = 2;
/// With a physics preset, the stage stays put while the parent piece is alive.
const STAYS_WITH_PARENT: u32 = 4;
/// A knife can break this stage.
const MELEE_BREAKS: u32 = 8;

/// Hits pass down this many levels of the tree at most.
const MAX_DEPTH: u32 = 20;

/// How a hit is handed on through the tree.
#[derive(Clone, Copy)]
pub(super) struct Strike {
    pub means: u8,
    /// Whether the pieces under the struck one take their share (fire does not pass any down).
    pub down: bool,
    /// A piece whose children are spared.
    pub spare: Option<usize>,
}

/// A blast's splash breaks past a stage that holds against everything else.
fn is_blast(means: u8) -> bool {
    matches!(means, T6_MOD_GRENADE_SPLASH | T6_MOD_PROJECTILE_SPLASH | T6_MOD_EXPLOSIVE)
}

/// The health a holding stage between `from` (exclusive) and `to` (inclusive) catches the piece at, if there is one.
/// The slot after the last real stage stands for "destroyed": its break health is the piece's `parent_damage_percent`
/// and its flags are the bits of its `explosive_damage_scale`.
fn caught_health(piece: &T5DestructiblePiece, from: Option<usize>, to: usize) -> Option<i16> {
    (from.map_or(0, |s| s + 1)..=to).find_map(|slot| {
        let (flags, share) = match piece.stages.get(slot) {
            Some(stage) => (stage.flags, stage.break_health),
            None => (piece.explosive_damage_scale.to_bits(), piece.parent_damage_percent),
        };
        (flags & HOLDS != 0).then(|| ((piece.health as f32 * share) as i16).wrapping_add(1))
    })
}

impl State {
    /// One hit of `amount` on the piece at `index`, by `means`; `down` hands the pieces under it their share, except
    /// under the `spare` piece. What it broke comes back.
    pub(super) fn hit_piece(&mut self, index: usize, amount: i32, means: u8, down: bool, spare: Option<usize>) -> Outcome {
        let mut out = Outcome::default();
        self.damage(index, amount, Strike { means, down, spare }, 0, &mut out);
        out
    }

    /// Applies `amount` to the piece at `index`, then to the pieces linked to it, noting what broke in `out`.
    pub(super) fn damage(&mut self, index: usize, amount: i32, strike: Strike, depth: u32, out: &mut Outcome) {
        if depth > MAX_DEPTH {
            diag::warn!(Sim, "T5 DamagePiece recursion limit: {}", self.definition.name);
            return;
        }
        let def = self.definition.clone();
        let piece = &def.pieces[index];
        let was = piece.stage(index, self.health[index]);
        let leaving = was.map(|slot| &piece.stages[slot]);
        let mut health = self.health[index]
            .wrapping_sub(amount.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
            .max(-1);
        let mut now = piece.stage(index, health);

        if self.hosted && strike.means == T6_MOD_MELEE && leaving.is_some_and(|s| s.flags & MELEE_BREAKS == 0) {
            return;
        }
        if !(self.hosted && is_blast(strike.means)) {
            if leaving.is_some_and(|s| s.flags & HOLDS != 0) {
                return;
            }
            // BO2 numbers a destroyed piece's stage past the last, and walks up to it too.
            let reach = match now {
                Some(slot) => Some(slot),
                None if self.hosted => was.map(|_| piece.stages.len()),
                None => None,
            };
            if let Some(caught) = reach.and_then(|to| caught_health(piece, was, to)) {
                health = caught;
                now = piece.stage(index, health);
            }
        }

        let parent = usize::from(piece.parent_piece);
        let moved = was != now;
        let pinned = leaving.is_some_and(|s| s.has_phys_preset && s.flags & STAYS_WITH_PARENT != 0);
        if moved && pinned && self.health.get(parent).is_some_and(|h| *h > 0) {
            return;
        }
        let hurts_parent = leaving.is_some_and(|s| s.flags & HURTS_PARENT != 0);
        if index == 0 && self.health[index] > 0 && health <= 0 {
            out.died = true;
        }
        self.health[index] = health;
        if moved {
            self.note_stage_change(piece, index, was, now, out);
        }

        if strike.down && strike.spare != Some(index) {
            for child in 0..self.health.len() {
                let kid = &def.pieces[child];
                if usize::from(kid.parent_piece) != index {
                    continue;
                }
                let share = (amount as f32 * kid.parent_damage_percent) as i32;
                if share != 0 {
                    self.damage(child, share, Strike { down: true, ..strike }, depth + 1, out);
                }
            }
        }
        if hurts_parent && parent < self.health.len() {
            let strike = Strike {
                down: true,
                spare: Some(index),
                ..strike
            };
            self.damage(parent, amount, strike, depth + 1, out);
        }
    }

    /// Records the effects of the piece at `index` going from stage `was` to stage `now`.
    fn note_stage_change(
        &self,
        piece: &T5DestructiblePiece,
        index: usize,
        was: Option<usize>,
        now: Option<usize>,
        out: &mut Outcome,
    ) {
        // The base piece's new stage plays its loop on the whole entity (a car's fire).
        if index == 0 {
            out.base_loop = Some(now.and_then(|slot| piece.stages[slot].loop_sound.clone()));
        }
        if let Some(first) = was {
            for stage in first..now.unwrap_or(piece.stages.len()) {
                if piece.stages[stage].show_bone.is_some() {
                    out.breaks.push(Break { piece: index, stage });
                }
            }
        }
        let delay = now.map_or(0.0, |slot| piece.stages[slot].max_time);
        if delay <= 0.0 {
            return;
        }
        if self.hosted {
            out.after.push((index, delay, piece.health));
        } else {
            diag::warn!(
                Sim,
                "T5 destructible breakafter callback unhosted: {} piece={}",
                self.definition.name,
                index
            );
        }
    }
}
