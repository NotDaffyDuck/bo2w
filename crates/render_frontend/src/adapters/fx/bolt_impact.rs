//! bo2mp: a crossbow bolt or a thrown blade striking (BO2's EV_BOLT_IMPACT,
//! played through CG_ImpactEffectForWeapon): the impact table's bolt row
//! (impact type 14) or blade row (15) for the surface it went into, a
//! player's body taking the first flesh effect. BO2 bolts a body hit's effect
//! to the bone the hit location names; our remote bodies carry no bone bolts
//! for effects (bullet hits play at the point too), so a body hit plays where
//! the bolt went in, turned to the hit normal, as BO2 does off a body.

use assets::StagedWeapons;
use bevy::prelude::*;
use render_fx::battle_fx::play_impact_table_room;
use render_fx::{
    EntityMarkBoard, EffectJournalCursor, EffectsSystemHandle, ReadyFxBook,
    PreparedFxElemTable, ReadyHitFx,
};

use crate::adapters::fx::{effect_play::EffectStage, world_decal::FrontendEffectStage};
use crate::prepare::scene::world::LevelStage;

/// The impact table is Black Ops II's only when it was loaded as such.
fn is_t6_table(impact_effect: Option<&ReadyHitFx>) -> bool {
    impact_effect
        .and_then(|prepared| prepared.0.as_ref())
        .is_some_and(|table| table.t6)
}

/// The surface normal a hit plays along; a hit without a direction faces up.
fn facing(direction: [f32; 3]) -> [f32; 3] {
    if direction == [0.0; 3] {
        [0.0, 0.0, 1.0]
    } else {
        direction
    }
}

/// The entity a mark is left on: a player struck, never the world itself.
fn marked_entity(other: impl TryInto<u16>) -> Option<u16> {
    let number: u16 = other.try_into().ok()?;
    (u32::from(number) < fx_t6::runtime::FX_ENTITYNUM_WORLD).then_some(number)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn play(
    hit: On<net::EntityBoltImpact>,
    weapons: Option<Res<StagedWeapons>>,
    effect_book: Option<Res<ReadyFxBook>>,
    impact_effect: Option<Res<ReadyHitFx>>,
    mut fx_elem_info: ResMut<PreparedFxElemTable>,
    mut host: ResMut<EffectsSystemHandle>,
    mut cursor: ResMut<EffectJournalCursor>,
    scene: Option<Res<LevelStage>>,
    marks: Res<EntityMarkBoard>,
) {
    let strike = hit.event.payload;
    let table_type = weapons
        .as_deref()
        .and_then(|prepared| prepared.0.summary_for(strike.weapon))
        .map(|facts| facts.impact_kind_id);
    // Black Ops II's impact table only.
    let (true, Some(impact_kind_id)) = (is_t6_table(impact_effect.as_deref()), table_type) else {
        return;
    };
    // A mark it leaves goes on the player it struck, as a bullet's does.
    let kept = std::mem::replace(
        &mut host.0.stain_target_entity,
        marked_entity(strike.other_entity_num),
    );
    let view = FrontendEffectStage::wrap(scene.as_deref(), &marks);
    play_impact_table_room(
        impact_kind_id,
        strike.origin,
        facing(strike.direction),
        strike.surf_type,
        0,
        0,
        effect_book.as_deref(),
        impact_effect.as_deref(),
        &mut fx_elem_info.0,
        &mut host,
        &mut cursor,
        view.as_ref().map(|s| s as &dyn EffectStage),
    );
    host.0.stain_target_entity = kept;
}
