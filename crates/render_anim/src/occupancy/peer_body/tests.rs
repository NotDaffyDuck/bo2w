#![cfg(test)]
use super::*;

#[test]
fn same_clients_ignores_order_and_repeats() {
    assert!(same_clients([3, 1, 2], [2, 3, 1]));
    assert!(same_clients([1, 1, 2], [2, 1]));
    assert!(same_clients([], []));
}

#[test]
fn same_clients_notices_a_difference() {
    assert!(!same_clients([1, 2], [1, 2, 3]));
    assert!(!same_clients([1], []));
    assert!(!same_clients([4], [5]));
}

#[test]
fn fx_bolts_start_empty() {
    let bolts = OtherBodyEffectTags::default();
    assert!(bolts.flash.is_none() && bolts.brass.is_none());
    assert!(bolts.knife.is_none() && bolts.laser.is_none());
}
