//! The player-state's ammo tables, 15 rows each keyed by ammo or clip index (key 0 is a
//! free row), and the weapon bytes that say whether a slot's gun is held in both hands.

/// Rows in each table, and weapon slots.
const ROWS: usize = 15;
/// A stock row: ammo index, then rounds not in a clip.
const AMMO_ROW_BYTES: usize = 8;
/// A clip row: clip index, then the right hand's rounds, then the left's.
const CLIP_ROW_BYTES: usize = 12;
/// Bytes per weapon slot in the weapon data.
const SLOT_BYTES: usize = 5;
/// The slot byte that is set while the gun is dual wielded.
const DUAL_WIELD_BYTE: usize = 1;
const WEAPON_DATA_BYTES: usize = ROWS * SLOT_BYTES;

/// The stock table's size.
pub const AMMO_TABLE_BYTES: usize = ROWS * AMMO_ROW_BYTES;
/// The clip table's size.
pub const AMMOCLIP_TABLE_BYTES: usize = ROWS * CLIP_ROW_BYTES;

fn read_i32(table: &[u8], at: usize) -> i32 {
    let mut word = [0; 4];
    word.copy_from_slice(&table[at..at + 4]);
    i32::from_le_bytes(word)
}

fn write_i32(table: &mut [u8], at: usize, value: i32) {
    table[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Where the row keyed `key` starts.
fn find_row(table: &[u8], row_bytes: usize, key: i32) -> Option<usize> {
    (0..ROWS)
        .map(|row| row * row_bytes)
        .find(|&at| read_i32(table, at) == key)
}

/// The row keyed `key`, else the first free row, given the key. None when full.
fn claim_row(table: &mut [u8], row_bytes: usize, key: i32) -> Option<usize> {
    let at = find_row(table, row_bytes, key).or_else(|| find_row(table, row_bytes, 0))?;
    write_i32(table, at, key);
    Some(at)
}

/// Where `hand`'s rounds sit in a clip row: 0 the right hand, anything else the left.
fn hand_field(hand: u8) -> usize {
    4 + usize::from(hand.min(1)) * 4
}

/// Rounds of `ammo_index` not in a clip; 0 with no row.
pub fn get_ammo_not_in_clip(ammo: &[u8; AMMO_TABLE_BYTES], ammo_index: i32) -> i32 {
    find_row(ammo, AMMO_ROW_BYTES, ammo_index).map_or(0, |at| read_i32(ammo, at + 4))
}

/// Sets the rounds of `ammo_index` not in a clip, taking a free row if it has none.
/// False when the table is full.
pub fn set_ammo_not_in_clip(
    ammo: &mut [u8; AMMO_TABLE_BYTES],
    ammo_index: i32,
    count: i32,
) -> bool {
    let Some(at) = claim_row(ammo, AMMO_ROW_BYTES, ammo_index) else {
        return false;
    };
    write_i32(ammo, at + 4, count);
    true
}

/// The clip table has a row for `clip_index` (0 never counts).
pub fn clip_row_present(ammoclip: &[u8; AMMOCLIP_TABLE_BYTES], clip_index: i32) -> bool {
    clip_index != 0 && find_row(ammoclip, CLIP_ROW_BYTES, clip_index).is_some()
}

/// The stock table has a row for `ammo_index` (0 never counts).
pub fn ammo_row_present(ammo: &[u8; AMMO_TABLE_BYTES], ammo_index: i32) -> bool {
    ammo_index != 0 && find_row(ammo, AMMO_ROW_BYTES, ammo_index).is_some()
}

/// Rounds in `hand`'s clip of `clip_index`; 0 with no row.
pub fn get_clip_for_hand(ammoclip: &[u8; AMMOCLIP_TABLE_BYTES], clip_index: i32, hand: u8) -> i32 {
    find_row(ammoclip, CLIP_ROW_BYTES, clip_index)
        .map_or(0, |at| read_i32(ammoclip, at + hand_field(hand)))
}

/// Sets the rounds in `hand`'s clip of `clip_index`, taking a free row if it has none.
/// False when the table is full.
pub fn set_clip_for_hand(
    ammoclip: &mut [u8; AMMOCLIP_TABLE_BYTES],
    clip_index: i32,
    hand: u8,
    count: i32,
) -> bool {
    let Some(at) = claim_row(ammoclip, CLIP_ROW_BYTES, clip_index) else {
        return false;
    };
    write_i32(ammoclip, at + hand_field(hand), count);
    true
}

/// The clip table key for a weapon: its clip index, or the weapon itself when that is 0.
#[must_use]
pub fn clip_table_key(clip_index: i32, weapon: u32) -> i32 {
    if clip_index == 0 {
        weapon as i32
    } else {
        clip_index
    }
}

/// The stock table key for a weapon: its ammo index, or the weapon itself when that is 0.
#[must_use]
pub fn ammo_table_key(ammo_index: i32, weapon: u32) -> i32 {
    if ammo_index == 0 {
        weapon as i32
    } else {
        ammo_index
    }
}

/// The slot holding `weapon`; -1 for none or no weapon.
pub fn player_weapons_find_slot(weapons: &[i32; 15], weapon: i32) -> i32 {
    if weapon == 0 {
        return -1;
    }
    weapons
        .iter()
        .position(|&held| held == weapon)
        .map_or(-1, |slot| slot as i32)
}

/// The dual wield byte of `weapon`'s slot, if it has one and the data covers it.
fn dual_wield_byte(weapons: &[i32; 15], weapon_data_len: usize, weapon: u32) -> Option<usize> {
    let slot = usize::try_from(player_weapons_find_slot(weapons, weapon as i32)).ok()?;
    (weapon_data_len >= WEAPON_DATA_BYTES).then_some(slot * SLOT_BYTES + DUAL_WIELD_BYTE)
}

/// Marks whether the slot holding `weapon` is dual wielded.
pub fn latch_weapon_dual_wield(
    weapons: &[i32; 15],
    weapon_data: &mut [u8],
    weapon: u32,
    dual: bool,
) {
    if let Some(at) = dual_wield_byte(weapons, weapon_data.len(), weapon) {
        weapon_data[at] = u8::from(dual);
    }
}

/// 1 when the held weapon is dual wielded, else 0.
pub fn num_hands_for_held(weapons: &[i32; 15], weapon_data: &[u8], held_weapon: u32) -> i32 {
    if held_weapon == 0 {
        return 0;
    }
    dual_wield_byte(weapons, weapon_data.len(), held_weapon)
        .map_or(0, |at| i32::from(weapon_data[at] != 0))
}

/// Every round a weapon has: its stock, its right clip, and its left clip when dual wielded.
pub fn get_ammo_player_both_clips(
    weapons: &[i32; 15],
    weapon_data: &[u8],
    ammo: &[u8; AMMO_TABLE_BYTES],
    ammoclip: &[u8; AMMOCLIP_TABLE_BYTES],
    weapon: u32,
    ammo_index: i32,
    clip_index: i32,
) -> i32 {
    let stock = get_ammo_not_in_clip(ammo, ammo_index);
    let right = get_clip_for_hand(ammoclip, clip_index, 0);
    let left = if num_hands_for_held(weapons, weapon_data, weapon) == 1 {
        get_clip_for_hand(ammoclip, clip_index, 1)
    } else {
        0
    };
    stock.wrapping_add(right).wrapping_add(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_rows_are_found_or_take_a_free_row() {
        let mut ammo = [0; AMMO_TABLE_BYTES];
        assert_eq!(get_ammo_not_in_clip(&ammo, 7), 0);
        assert!(!ammo_row_present(&ammo, 7));
        assert!(set_ammo_not_in_clip(&mut ammo, 7, 90));
        assert!(set_ammo_not_in_clip(&mut ammo, 7, 60));
        assert_eq!(get_ammo_not_in_clip(&ammo, 7), 60);
        assert!(ammo_row_present(&ammo, 7));
        assert!(!ammo_row_present(&ammo, 0));
        assert_eq!(&ammo[..8], &[7, 0, 0, 0, 60, 0, 0, 0]);
    }

    #[test]
    fn a_full_table_takes_no_new_key() {
        let mut ammo = [0; AMMO_TABLE_BYTES];
        for key in 1..=15 {
            assert!(set_ammo_not_in_clip(&mut ammo, key, key));
        }
        assert!(!set_ammo_not_in_clip(&mut ammo, 16, 1));
        assert!(set_ammo_not_in_clip(&mut ammo, 15, 2));
        assert_eq!(get_ammo_not_in_clip(&ammo, 15), 2);
    }

    #[test]
    fn each_hand_has_its_own_clip() {
        let mut clips = [0; AMMOCLIP_TABLE_BYTES];
        assert!(set_clip_for_hand(&mut clips, 4, 0, 30));
        assert!(set_clip_for_hand(&mut clips, 4, 1, 25));
        assert!(set_clip_for_hand(&mut clips, 4, 9, 20));
        assert_eq!(get_clip_for_hand(&clips, 4, 0), 30);
        assert_eq!(get_clip_for_hand(&clips, 4, 1), 20);
        assert!(clip_row_present(&clips, 4));
        assert!(!clip_row_present(&clips, 5));
        assert_eq!(get_clip_for_hand(&clips, 5, 0), 0);
    }

    #[test]
    fn a_weapon_with_no_index_keys_by_itself() {
        assert_eq!(clip_table_key(0, 12), 12);
        assert_eq!(clip_table_key(3, 12), 3);
        assert_eq!(ammo_table_key(0, 12), 12);
        assert_eq!(ammo_table_key(3, 12), 3);
    }

    #[test]
    fn the_left_clip_counts_only_when_dual_wielded() {
        let mut weapons = [0; 15];
        weapons[2] = 40;
        let mut data = [0u8; WEAPON_DATA_BYTES];
        let mut ammo = [0; AMMO_TABLE_BYTES];
        let mut clips = [0; AMMOCLIP_TABLE_BYTES];
        set_ammo_not_in_clip(&mut ammo, 40, 100);
        set_clip_for_hand(&mut clips, 40, 0, 10);
        set_clip_for_hand(&mut clips, 40, 1, 7);

        assert_eq!(player_weapons_find_slot(&weapons, 40), 2);
        assert_eq!(player_weapons_find_slot(&weapons, 0), -1);
        assert_eq!(player_weapons_find_slot(&weapons, 41), -1);
        assert_eq!(
            get_ammo_player_both_clips(&weapons, &data, &ammo, &clips, 40, 40, 40),
            110
        );

        latch_weapon_dual_wield(&weapons, &mut data, 40, true);
        assert_eq!(data[2 * 5 + 1], 1);
        assert_eq!(num_hands_for_held(&weapons, &data, 40), 1);
        assert_eq!(num_hands_for_held(&weapons, &data[..10], 40), 0);
        assert_eq!(
            get_ammo_player_both_clips(&weapons, &data, &ammo, &clips, 40, 40, 40),
            117
        );
    }
}
