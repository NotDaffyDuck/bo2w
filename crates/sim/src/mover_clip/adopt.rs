use crate::mover_clip::{Placement, is_hidden, movers_by_id};
use crate::ServerModelSource;
use crate::shot_trace::{EntityHitAbilities, AttachedSolidShape};
use crate::ent_slots::MoverSlot;
use entity_t6::script_mover::SOLID_BMODEL;

/// What one adoption changed.
#[derive(Clone, Copy, Default)]
struct Churn {
    made: usize,
    removed: usize,
}

/// Gives the prediction world a collision row for each brush mover that has none (a door or debris pile whose level
/// started after prediction armed). The row holds the brush model the mover names, at the mover's pose, and is marked
/// as made from a snapshot: it goes when its mover leaves the snapshot, and `follow_movers` keeps its pose and
/// solidity from then on.
pub(crate) fn adopt_brush_movers(
    rows: &mut Vec<EntityHitAbilities>,
    movers: &[MoverSlot],
    clock_stamp_ms: i32,
) {
    let mut brush_movers = movers_by_id(movers);
    brush_movers.retain(|_, mover| mover.state.solid == SOLID_BMODEL && mover.state.index >= 0);

    let before = rows.len();
    rows.retain(|row| {
        let still_there = row.owner.scripted_model().is_some_and(|id| brush_movers.contains_key(&id));
        still_there || !row.from_world_state
    });
    let mut churn = Churn { removed: before - rows.len(), made: 0 };

    for (id, mover) in brush_movers {
        let owner = ServerModelSource::ScriptModel(id);
        if rows.binary_search_by_key(&owner, |row| row.owner).is_ok() {
            continue;
        }
        rows.push(row_for(owner, mover, Placement::of(mover, clock_stamp_ms)));
        churn.made += 1;
    }
    if churn.made > 0 {
        rows.sort_by_key(|row| row.owner);
    }
    log_churn(churn, rows);
}

fn row_for(owner: ServerModelSource, mover: &MoverSlot, pose: Placement) -> EntityHitAbilities {
    let brush = AttachedSolidShape { collision_model_ref: mover.state.index as u32, origin: pose.at, turn: pose.turn };
    let mut row = EntityHitAbilities::now_tick(owner, None, vec![brush]);
    row.from_world_state = true;
    row.unseen = is_hidden(mover);
    row.solid = !mover.nonsolid;
    row.followed_pose = Some((pose.at, pose.turn));
    row
}

/// Reports a change when `BO2MP_PRED_LOG` is set.
fn log_churn(churn: Churn, rows: &[EntityHitAbilities]) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let changed = churn.made + churn.removed > 0;
    if changed && *ENABLED.get_or_init(|| std::env::var_os("BO2MP_PRED_LOG").is_some()) {
        let total = rows.iter().filter(|row| row.from_world_state).count();
        diag::info!(Sim, "prediction brush rows: {} made, {} dropped, {total} in all", churn.made, churn.removed);
    }
}
