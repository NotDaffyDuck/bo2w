//! bo2mp gun brain: BO2's per-weapon viewmodel facts (anim lengths, blend times, sprint
//! and ADS numbers), staged at match load so the first-person gun can play BO2's anims.
//! The table is keyed by this game's weapon index, like the sim's gun facts.

use std::sync::Arc;

use weapon_t6::facts::FactsTable;
use weapon_t6::viewmodel::VmTable;

#[derive(bevy_ecs::prelude::Resource, Clone, Default)]
pub struct T6VmTableRes(pub Arc<VmTable>);

/// The same gun facts the sim runs on, for the client's view kick (BO2 kicks the view on the
/// client from these numbers).
#[derive(bevy_ecs::prelude::Resource, Clone, Default)]
pub struct T6GunFactsRes(pub Arc<FactsTable>);
