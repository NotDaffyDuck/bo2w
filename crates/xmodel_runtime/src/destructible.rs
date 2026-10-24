//! What a destructible model is made of: pieces that wear through stages as
//! they take damage, and the debris a stage throws off when it breaks.

use crate::HidePartBits;

/// A destructible model: its name, the model it dresses, and its pieces.
#[derive(Clone, Debug, PartialEq)]
pub struct T5DestructibleDef {
    pub name: String,
    pub model: String,
    pub pieces: Vec<T5DestructiblePiece>,
    pub client_only: bool,
}

/// One breakable part. It walks through up to five stages as its health drops.
#[derive(Clone, Debug, PartialEq)]
pub struct T5DestructiblePiece {
    pub stages: [T5DestructibleStage; 5],
    pub parent_piece: u8,
    pub parent_damage_percent: f32,
    pub bullet_damage_scale: f32,
    pub explosive_damage_scale: f32,
    pub melee_damage_scale: f32,
    /// Scale for an impact hit (a thrown grenade or a bolt striking it).
    pub impact_damage_scale: f32,
    /// Share of a hit on this piece that the entity's own health loses; 0
    /// keeps the entity whole.
    pub entity_damage_transfer: f32,
    pub health: i32,
    /// Bones each stage hides, as bit words over the model's bones.
    pub hide_bones: [u32; 5],
}

/// One step of a piece's wear: the bone it shows, when it gives way and what
/// it throws off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T5DestructibleStage {
    pub show_bone: Option<String>,
    pub break_health: f32,
    pub max_time: f32,
    pub flags: u32,
    pub break_effect: Option<String>,
    pub break_sound: Option<String>,
    pub break_notify: Option<String>,
    pub loop_sound: Option<String>,
    pub has_phys_preset: bool,
    pub spawn_models: [Option<String>; 3],
    /// How each spawn model flies once it comes loose (its model's physics
    /// preset); none leaves that piece unspawned.
    pub spawn_presets: [Option<DebrisPreset>; 3],
}

/// The parts of a physics preset a broken-off piece flies by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebrisPreset {
    pub mass: f32,
    pub bounce: f32,
    pub friction: f32,
    pub bullet_force_scale: f32,
    pub explosive_force_scale: f32,
    pub gravity_scale: f32,
}

impl T5DestructiblePiece {
    /// The stage this piece shows at `health`. Walking the stages in order,
    /// the first whose break threshold the health still meets is the answer
    /// if it shows a bone. A stage with no bone ends the walk: the first
    /// piece falls back to the stage before it, the others show nothing.
    pub fn stage(&self, index: usize, health: i16) -> Option<usize> {
        for (at, stage) in self.stages.iter().enumerate() {
            if health as f32 >= self.health as f32 * stage.break_health {
                return stage.show_bone.as_ref().map(|_| at);
            }
            if stage.show_bone.is_none() {
                return if index == 0 { at.checked_sub(1) } else { None };
            }
        }
        None
    }
}

impl T5DestructibleDef {
    /// The bones to hide at the given per-piece health: every bone any piece
    /// hides, minus the bone each piece shows at its current stage.
    pub fn hide_parts(&self, health: &[i16], bone_names: &[String]) -> HidePartBits {
        assert_eq!(health.len(), self.pieces.len());
        let mut words = [0u32; HidePartBits::WORDS];
        for piece in &self.pieces {
            for (word, hide) in words.iter_mut().zip(piece.hide_bones) {
                *word |= hide;
            }
        }
        for (index, piece) in self.pieces.iter().enumerate() {
            if let Some(stage) = piece.stage(index, health[index])
                && let Some(bone) = bone_names
                    .iter()
                    .position(|name| Some(name) == piece.stages[stage].show_bone.as_ref())
            {
                words[bone / 32] &= !(0x8000_0000 >> (bone % 32));
            }
        }
        HidePartBits::from_words(words)
    }
}
