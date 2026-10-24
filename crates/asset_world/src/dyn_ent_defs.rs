//! The dynamic entities a map defines, and the physics preset each one uses.

use crate::dyn_ent_kind::DynEntType;

/// A physics preset: how heavy a prop is, how it bounces and slides, how hard
/// bullets and blasts push it, and what it sounds like when it breaks.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedPhysPreset {
    pub name: String,
    pub preset_type: i32,
    pub mass: f32,
    pub bounce: f32,
    pub friction: f32,
    pub bullet_force_scale: f32,
    pub explosive_force_scale: f32,
    pub snd_alias_prefix: String,
    pub pieces_spread_fraction: f32,
    pub pieces_upward_velocity: f32,
    pub temp_default_to_cylinder: bool,
    pub per_surface_snd_alias: bool,
}

/// One dynamic entity of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct DynEntDef {
    /// Its row in the map's list.
    pub index: u16,
    pub ty: DynEntType,
    pub quat: [f32; 4],
    pub origin: [f32; 3],
    /// The model a model entity draws.
    pub xmodel: Option<String>,
    /// The brush model a brush entity draws.
    pub brush_model: u16,
    /// The effect played when the entity breaks.
    pub destroy_fx: Option<String>,
    pub phys_preset: Option<OwnedPhysPreset>,
    pub health: i32,
}

/// A map's dynamic entities, split by what they draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DynEntCatalog {
    pub models: Vec<DynEntDef>,
    pub brushes: Vec<DynEntDef>,
}

impl DynEntCatalog {
    fn count_of(&self, ty: DynEntType) -> usize {
        self.models
            .iter()
            .chain(&self.brushes)
            .filter(|def| def.ty == ty)
            .count()
    }

    /// A one-line summary for the load report.
    pub fn report_line(&self) -> String {
        format!(
            "dynents: models={} brushes={} clutter={} destruct={}",
            self.models.len(),
            self.brushes.len(),
            self.count_of(DynEntType::Clutter),
            self.count_of(DynEntType::Destruct),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(ty: DynEntType) -> DynEntDef {
        DynEntDef {
            index: 0,
            ty,
            quat: [0.0, 0.0, 0.0, 1.0],
            origin: [0.0; 3],
            xmodel: None,
            brush_model: 0,
            destroy_fx: None,
            phys_preset: None,
            health: 0,
        }
    }

    #[test]
    fn the_report_counts_both_lists_by_kind() {
        let catalog = DynEntCatalog {
            models: vec![def(DynEntType::Clutter), def(DynEntType::Destruct)],
            brushes: vec![def(DynEntType::Clutter)],
        };
        assert_eq!(catalog.report_line(), "dynents: models=2 brushes=1 clutter=2 destruct=1");
    }

    #[test]
    fn an_empty_catalog_reports_zeroes() {
        assert_eq!(
            DynEntCatalog::default().report_line(),
            "dynents: models=0 brushes=0 clutter=0 destruct=0"
        );
    }
}
