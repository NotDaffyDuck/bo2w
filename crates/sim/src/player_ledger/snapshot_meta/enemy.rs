use crate::sim_parts::SeatNo;
use entity_t6::team;

/// Bit flipped in a composition's revision when its base model is swapped, so a cached build is not reused.
const REVISION_SWAP_BIT: u32 = 0x8000_0000;

impl super::TickLedgerNotes {
    /// Dresses the script models for `viewer`. A model shows its enemy look to everyone it does not belong to or
    /// share a side with. The rows are consumed.
    pub fn swap_enemy_models(&mut self, viewer: SeatNo) {
        if self.enemy_models.is_empty() {
            return;
        }
        let viewer_team = self.for_client(viewer).map_or(team::FREE, |meta| meta.client_state_team);
        let rows = std::mem::take(&mut self.enemy_models);
        for row in rows.iter().filter(|row| !row.shows_own_look_to(viewer, viewer_team)) {
            let owned = self.entity_dobjs.iter_mut().filter(|(owner, _)| owner.scripted_model() == Some(row.id));
            for (_, state) in owned {
                let composition = &mut state.composition;
                let Some(base) = composition.models.first_mut() else { continue };
                if base.model != row.mesh {
                    base.model.clone_from(&row.mesh);
                    composition.revision ^= REVISION_SWAP_BIT;
                }
            }
        }
    }
}
