use fx_t6::runtime::impact_table_row;

/// The impact types whose strike reaches the client as a bullet-hit event
/// (`EV_BULLET_HIT`, `EV_BULLET_HIT_SHIELD`), so it needs no trace of its own.
///
/// TODO(fact): these are the numbers `impact_table_row` uses (bullets 1, 2
/// and 9, shield 3); BO2's impact type names for them are not in the facts.
const BULLET_HIT_EVENT_IMPACT_TYPES: [i32; 4] = [1, 2, 3, 9];

pub fn fire_weapon_fx_should_client_trace(impact_type: i32) -> bool {
    impact_table_row(impact_type, false).is_some()
        && !BULLET_HIT_EVENT_IMPACT_TYPES.contains(&impact_type)
}
