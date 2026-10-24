//! The breakable windows of a Black Ops II map.

use crate::{T6GlassPaneDraw, T6GlassPlacement};

/// The fewest outline points that make a pane. The map's own panes have at
/// most sixteen.
const MIN_OUTLINE_POINTS: usize = 3;

/// A map's windows. Window `i` is the same window in `panes` and `server`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6Glass {
    /// How each window looks while whole, from its own surfaces. Empty until
    /// the world's surfaces are loaded.
    pub panes: Vec<T6GlassPaneDraw>,
    /// Where each window stands and how much it takes to break it.
    pub server: Vec<T6GlassServerPane>,
    /// Each window definition the map uses, once, in the order they first
    /// appear: its whole-pane material and its shard material.
    pub def_materials: Vec<(String, String)>,
}

/// What the game logic knows of one window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6GlassServerPane {
    pub origin: [f32; 3],
    /// The corners of the box that holds the window in the world.
    pub absmin: [f32; 3],
    pub absmax: [f32; 3],
    /// The health the window's definition gives it.
    pub max_health: i32,
}

impl T6Glass {
    pub fn report_line(&self) -> String {
        format!(
            "panes={} defs={} drawn={}",
            self.server.len(),
            self.def_materials.len(),
            self.panes.len()
        )
    }
}

/// The windows the map lists, with, for each, where the model holding its
/// clip brushes stands. A window with too few outline points to make a pane
/// is left out, and a map left with no windows gives `None`.
pub fn build_t6_glass(glasses: &[asset_t6::GlassRef]) -> Option<(T6Glass, Vec<T6GlassPlacement>)> {
    let mut glass = T6Glass::default();
    let mut placements = Vec::new();
    let mut known_defs: Vec<&str> = Vec::new();
    let usable = glasses
        .iter()
        .enumerate()
        .filter(|(_, window)| window.outline.len() >= MIN_OUTLINE_POINTS);
    for (index, window) in usable {
        if !known_defs.contains(&window.def_name.as_str()) {
            known_defs.push(&window.def_name);
            glass
                .def_materials
                .push((window.pristine_material.clone(), window.shard_material.clone()));
        }
        placements.push(T6GlassPlacement {
            glass: index,
            brush_model: window.brush_model,
            origin: window.origin,
            angles: window.angles,
        });
        glass.server.push(T6GlassServerPane {
            origin: window.origin,
            absmin: window.absmin,
            absmax: window.absmax,
            max_health: window.max_health,
        });
    }
    (!glass.server.is_empty()).then_some((glass, placements))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(def: &str, points: usize, health: i32) -> asset_t6::GlassRef {
        asset_t6::GlassRef {
            def_name: def.into(),
            outline: vec![[0.0, 0.0]; points],
            pristine_material: format!("{def}_whole"),
            shard_material: format!("{def}_shard"),
            max_health: health,
            brush_model: points as u32,
            ..Default::default()
        }
    }

    #[test]
    fn windows_too_small_to_make_a_pane_are_left_out() {
        let (glass, placements) =
            build_t6_glass(&[window("a", 2, 10), window("a", 3, 20), window("a", 4, 30)]).unwrap();
        assert_eq!(glass.server.len(), 2);
        assert_eq!(glass.server[0].max_health, 20);
        let kept: Vec<usize> = placements.iter().map(|place| place.glass).collect();
        assert_eq!(kept, [1, 2]);
        assert_eq!(placements[1].brush_model, 4);
    }

    #[test]
    fn each_definition_lists_its_materials_once() {
        let (glass, _) =
            build_t6_glass(&[window("a", 3, 1), window("b", 3, 1), window("a", 3, 1)]).unwrap();
        assert_eq!(
            glass.def_materials,
            [("a_whole".into(), "a_shard".into()), ("b_whole".into(), "b_shard".into())]
        );
        assert_eq!(glass.report_line(), "panes=3 defs=2 drawn=0");
    }

    #[test]
    fn a_map_with_no_usable_window_has_no_glass() {
        assert!(build_t6_glass(&[]).is_none());
        assert!(build_t6_glass(&[window("a", 1, 5)]).is_none());
    }
}
