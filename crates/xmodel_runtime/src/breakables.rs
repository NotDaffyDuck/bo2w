//! Destructible models. A model carries pieces; each piece wears through up
//! to five stages as its health falls, and a stage that breaks can throw
//! debris off.

use crate::HiddenParts;

/// Number of wear stages every piece has, used or not.
const STAGES_PER_PIECE: usize = 5;
/// Number of debris models one stage can spawn.
const DEBRIS_SLOTS: usize = 3;

/// A destructible: the model it dresses and the pieces that can break.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakableModelDef {
    pub name: String,
    pub model: String,
    pub local_machine_only: bool,
    pub pieces: Vec<BreakableChunk>,
}

/// One breakable part of a destructible.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakableChunk {
    pub health: i32,
    pub stages: [BreakableStage; STAGES_PER_PIECE],
    /// Bones this piece may hide, as bit words over the model's bones (bone
    /// `n` is bit `31 - n % 32` of word `n / 32`).
    pub conceal_joints: [u32; HiddenParts::BIT_CHUNK_COUNT],
    pub owner_fragment: u8,
    pub inherited_harm_ratio: f32,
    pub round_damage_multiplier: f32,
    pub blast_damage_multiplier: f32,
    pub melee_damage_scale: f32,
    /// Scale for an impact hit, such as a thrown grenade or a bolt.
    pub impact_damage_scale: f32,
    /// Share of a hit on this piece that comes off the entity's own health.
    /// Zero leaves the entity whole.
    pub entity_damage_transfer: f32,
}

/// One step of a piece's wear.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BreakableStage {
    /// Bone shown while the piece sits in this stage.
    pub reveal_joint: Option<String>,
    /// Fraction of the piece's health below which the stage gives way.
    pub shatter_hitpoints: f32,
    pub longest_duration: f32,
    pub flags: u32,
    pub uses_physics_preset: bool,
    pub shatter_visual: Option<String>,
    pub shatter_audio: Option<String>,
    pub shatter_signal: Option<String>,
    pub repeating_audio: Option<String>,
    pub debris_models: [Option<String>; DEBRIS_SLOTS],
    /// How each debris model flies once loose; `None` spawns nothing in
    /// that slot.
    pub spawn_presets: [Option<DebrisPreset>; DEBRIS_SLOTS],
}

/// Flight parameters of a piece that has broken off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebrisPreset {
    pub mass: f32,
    pub bounce: f32,
    pub friction: f32,
    pub gravity_scale: f32,
    pub bullet_push: f32,
    pub blast_push: f32,
}

impl BreakableChunk {
    /// Index of the stage this piece shows at `health`, if any.
    ///
    /// The stages are tried in order and the first one whose threshold the
    /// health still reaches decides, provided it shows a bone. A stage that
    /// shows no bone ends the search early: the first piece of a model then
    /// falls back to the stage before it, any other piece shows nothing.
    pub fn stage(&self, piece_index: usize, health: i16) -> Option<usize> {
        let current = f32::from(health);
        let full = self.health as f32;
        for (at, stage) in self.stages.iter().enumerate() {
            let reached = current >= full * stage.shatter_hitpoints;
            match (reached, stage.reveal_joint.is_some()) {
                (true, shown) => return shown.then_some(at),
                (false, false) => return if piece_index == 0 { at.checked_sub(1) } else { None },
                (false, true) => {}
            }
        }
        None
    }
}

impl BreakableModelDef {
    /// Bones to hide for the given per-piece health: everything any piece can
    /// hide, except the bone each piece shows right now.
    pub fn hidden_parts(&self, piece_hp: &[i16], joint_labels: &[String]) -> HiddenParts {
        assert_eq!(piece_hp.len(), self.pieces.len());
        let mut words = self
            .pieces
            .iter()
            .map(|piece| piece.conceal_joints)
            .fold([0u32; HiddenParts::BIT_CHUNK_COUNT], |mut all, piece| {
                for (word, bits) in all.iter_mut().zip(piece) {
                    *word |= bits;
                }
                all
            });
        for (index, (piece, &hp)) in self.pieces.iter().zip(piece_hp).enumerate() {
            let Some(stage) = piece.stage(index, hp) else {
                continue;
            };
            let shown = piece.stages[stage].reveal_joint.as_ref();
            if let Some(bone) = joint_labels.iter().position(|name| Some(name) == shown) {
                words[bone / 32] &= !(0x8000_0000u32 >> (bone % 32));
            }
        }
        HiddenParts::from_bit_chunks(words)
    }
}
