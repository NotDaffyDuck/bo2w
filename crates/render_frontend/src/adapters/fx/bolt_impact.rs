//! bo2mp: a crossbow bolt or a thrown blade striking (BO2's EV_BOLT_IMPACT,
//! played through CG_ImpactEffectForWeapon): the impact table's bolt row
//! (impact type 14) or blade row (15) for the surface it went into, a
//! player's body taking the first flesh effect. BO2 bolts a body hit's effect
//! to the bone the hit location names; our remote bodies carry no bone bolts
//! for effects (bullet hits play at the point too), so a body hit plays where
//! the bolt went in, turned to the hit normal, as BO2 does off a body.

use assets::PreparedWeapons;
use bevy::prelude::*;
use render_fx::combat::play_impact_table_cell;
use render_fx::{
    CombatFxDump, EntityMarks, FxJournalCursor, HostFxSystem, PreparedFxCatalog,
    PreparedFxElemInfos, PreparedImpactFx,
};

use crate::adapters::fx::{present::FxScene, world_mark::FrontendFxScene};
use crate::prepare::scene::world::WorldScene;

#[allow(clippy::too_many_arguments)]
pub(crate) fn play(
    hit: On<net::EntityBoltImpact>,
    weapons: Option<Res<PreparedWeapons>>,
    catalog: Option<Res<PreparedFxCatalog>>,
    impact_fx: Option<Res<PreparedImpactFx>>,
    mut elem_infos: ResMut<PreparedFxElemInfos>,
    mut host: ResMut<HostFxSystem>,
    mut cursor: ResMut<FxJournalCursor>,
    mut combat: ResMut<CombatFxDump>,
    scene: Option<Res<WorldScene>>,
    marks: Res<EntityMarks>,
) {
    // Black Ops II's impact table only.
    if !impact_fx
        .as_ref()
        .and_then(|fx| fx.0.as_ref())
        .is_some_and(|t| t.t6)
    {
        return;
    }
    let p = hit.event.payload;
    let Some(impact_type) = weapons
        .as_deref()
        .and_then(|w| w.0.facts_of(p.weapon))
        .map(|f| f.impact_type)
    else {
        return;
    };
    let normal = if p.direction == [0.0, 0.0, 0.0] {
        [0.0, 0.0, 1.0]
    } else {
        p.direction
    };
    // A mark it leaves goes on the player it struck, as a bullet's does.
    let previous_mark_entity = host.0.spawn_mark_entity;
    host.0.spawn_mark_entity = u16::try_from(p.other_entity_num)
        .ok()
        .filter(|&n| u32::from(n) < fx_t6::runtime::FX_ENTITYNUM_WORLD);
    let view = FrontendFxScene::wrap(scene.as_deref(), &marks);
    play_impact_table_cell(
        impact_type,
        p.origin,
        normal,
        p.surf_type,
        0,
        0,
        catalog.as_deref(),
        impact_fx.as_deref(),
        &mut elem_infos.0,
        &mut host,
        &mut cursor,
        &mut combat,
        view.as_ref().map(|s| s as &dyn FxScene),
    );
    host.0.spawn_mark_entity = previous_mark_entity;
}
