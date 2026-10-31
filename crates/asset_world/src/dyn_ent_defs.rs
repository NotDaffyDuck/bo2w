//! The dynamic entities a map defines, and the physics preset each one uses.

use crate::dyn_ent_kind::MovableObjectKind;

/// A physics preset: how heavy a prop is, how it bounces and slides, how hard
/// bullets and blasts push it, and what it sounds like when it breaks.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfContainedPhysicsPreset {
    pub name: String,
    pub template_category: i32,
    pub mass: f32,
    pub bounce: f32,
    pub friction: f32,
    pub shot_push_gain: f32,
    pub blast_push_gain: f32,
    pub audio_entry_stem: String,
    pub shard_scatter_ratio: f32,
    pub shard_rise_speed: f32,
    pub falls_back_to_tube: bool,
    pub material_footstep_entry: bool,
}

/// One dynamic entity of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct MovableObjectDef {
    /// Its row in the map's list.
    pub index: u16,
    pub ty: MovableObjectKind,
    pub turn_quat: [f32; 4],
    pub origin: [f32; 3],
    /// The model a model entity draws.
    pub model_name: Option<String>,
    /// The brush model a brush entity draws.
    pub solid_model: u16,
    /// The effect played when the entity breaks.
    pub destroy_effect: Option<String>,
    pub physics_template: Option<SelfContainedPhysicsPreset>,
    pub hit_points: i32,
}

/// A map's dynamic entities, split by what they draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MovableObjectRoster {
    pub models: Vec<MovableObjectDef>,
    pub brushes: Vec<MovableObjectDef>,
}

impl MovableObjectRoster {
    fn count_of(&self, ty: MovableObjectKind) -> usize {
        self.models
            .iter()
            .chain(&self.brushes)
            .filter(|entry| entry.ty == ty)
            .count()
    }

    /// A one-line summary for the load report.
    pub fn summary_text(&self) -> String {
        format!(
            "dynents: models={} brushes={} clutter={} destruct={}",
            self.models.len(),
            self.brushes.len(),
            self.count_of(MovableObjectKind::Clutter),
            self.count_of(MovableObjectKind::Destruct),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(ty: MovableObjectKind) -> MovableObjectDef {
        MovableObjectDef {
            index: 0,
            ty,
            turn_quat: [0.0, 0.0, 0.0, 1.0],
            origin: [0.0; 3],
            model_name: None,
            solid_model: 0,
            destroy_effect: None,
            physics_template: None,
            hit_points: 0,
        }
    }

    #[test]
    fn the_report_counts_both_lists_by_kind() {
        let catalog = MovableObjectRoster {
            models: vec![def(MovableObjectKind::Clutter), def(MovableObjectKind::Destruct)],
            brushes: vec![def(MovableObjectKind::Clutter)],
        };
        assert_eq!(catalog.summary_text(), "dynents: models=2 brushes=1 clutter=2 destruct=1");
    }

    #[test]
    fn an_empty_catalog_reports_zeroes() {
        assert_eq!(
            MovableObjectRoster::default().summary_text(),
            "dynents: models=0 brushes=0 clutter=0 destruct=0"
        );
    }
}
