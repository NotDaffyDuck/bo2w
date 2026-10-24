//! What loading a match settles about its map before the world is installed.

use std::sync::Arc;

use asset_material::MaterialDefinitions;

use crate::t6_hud_assets::{T6HudFonts, T6HudIcons, T6Ui};

/// The one pool of materials a prepared match owns. Geometry never holds a
/// pool of its own: it holds indices into the map zone's list, and
/// `map_ids` turns those into rows of `population`.
#[derive(Clone, Default)]
pub struct MatchMaterials {
    pub population: Arc<MaterialDefinitions>,
    pub common_profile_id: u64,
    pub products_id: u64,
    /// Row in `population`, for each material index local to the map zone.
    pub map_ids: Vec<Option<usize>>,
}

/// What the map says about the match it hosts. The lane collects these once
/// and hands them on to their single owner, [`PreparedMap`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapFacts {
    pub north_yaw: Option<f32>,
    pub airstrike_height: Option<f32>,
    pub script_sound: asset_audio::MapScriptSoundFacts,
    pub team_settings: asset_game::MapTeamSettings,
    /// The Zombies HUD's pictures: perk icons, timed power-ups.
    pub t6_hud_icons: T6HudIcons,
    pub t6_hud_fonts: T6HudFonts,
    /// Black Ops II's own UI scripts and what they read.
    pub t6_ui: T6Ui,
    /// The third-person player animation script and its anim types. Every
    /// client poses the other players with it.
    pub t6_playeranim: Option<(String, String)>,
    /// The ragdoll definitions (`ragdoll.cfg`).
    pub t6_ragdoll: Option<String>,
    /// How fast each player animation moves, in units per second taken from
    /// its root motion. A legs animation plays at the player's speed over
    /// this.
    pub t6_anim_speeds: Vec<(String, f32)>,
    /// The map's weapons as raw facts, in the `weapon_t6` dump format and
    /// keyed by name.
    pub t6_gun_facts: String,
}

/// The map a match was prepared for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedMap {
    pub zone: String,
    pub namespace: Option<asset_core::AssetNamespace>,
    pub spawns: Vec<asset_world::SpawnPoint>,
    pub facts: MapFacts,
}
