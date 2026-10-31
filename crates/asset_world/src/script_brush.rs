//! Script-driven brush models: the ids scripts know them by and the
//! placements the map authors for them.

/// The id a script-placed model goes by. It is the entity's position in the
/// map's entity text, so it stays the same from one load to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptModelId(pub(crate) u32);

impl ScriptModelId {
    pub const fn from_source_ordinal(n: u32) -> Self {
        Self(n)
    }

    pub const fn origin_sequence_number(self) -> u32 {
        self.0
    }
}

/// A `script_brushmodel` the map places: which collision submodel it stands
/// for, and where it sits before a script moves it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptSolidPlacementInfo {
    /// Position of the entity in the map's entity text.
    pub origin_sequence_number: u32,
    /// The submodel it draws and collides as; 0 is the world itself.
    pub collision_model_ref: u32,
    pub origin: [f32; 3],
    /// Pitch, yaw, roll in degrees.
    pub angles: [f32; 3],
    /// The `scripted_detonator` key scripts use to find it; empty when unset.
    pub scripted_detonator: String,
    /// The `script_gameobject` key; empty when unset.
    pub script_entity: String,
    /// The `targetname` key; empty when unset.
    pub lookup_key: String,
}

/// How a script model is tied to a brush model.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ScriptSolidBinding {
    #[default]
    None,
    /// Tied to exactly one brush model.
    Linked(ScriptSolidPlacementInfo),
    /// The script's target names more than one brush model.
    Ambiguous { target_name: String, linked_count: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_id_round_trips_its_ordinal() {
        assert_eq!(ScriptModelId::from_source_ordinal(41).origin_sequence_number(), 41);
        assert!(ScriptModelId::from_source_ordinal(1) < ScriptModelId::from_source_ordinal(2));
    }

    #[test]
    fn a_model_starts_with_no_brush_link() {
        assert_eq!(ScriptSolidBinding::default(), ScriptSolidBinding::None);
    }
}
