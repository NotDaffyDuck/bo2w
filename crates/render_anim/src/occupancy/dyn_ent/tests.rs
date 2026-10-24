use super::*;

fn bits_with(word_count: usize, cell_count: usize, set: &[(usize, u32)]) -> DynEntCellBits {
    let mut table = DynEntCellBits::default();
    table.ensure((word_count * 32) as u32, cell_count);
    for &(cell, id) in set {
        let word = cell * table.word_count + (id as usize >> 5);
        table.bits[word] |= 0x8000_0000 >> (id & 31);
    }
    table
}

#[test]
fn cell_bits_rebuild_only_when_the_shape_changes() {
    let mut table = DynEntCellBits::default();
    assert!(!table.is_ready());
    assert!(table.ensure(40, 3));
    assert_eq!(
        (table.word_count, table.cell_count, table.bits.len()),
        (2, 3, 6)
    );
    assert!(table.is_ready());
    table.bits[0] = 7;
    assert!(!table.ensure(40, 3));
    assert_eq!(table.bits[0], 7);
    assert!(table.ensure(41, 3));
    assert_eq!(table.bits[0], 0);
}

#[test]
fn hull_needs_finite_positive_extents() {
    let hull = hull_from_bounds(Some(([1.0, 2.0, 3.0], [1.0, 2.0, 0.5])));
    assert_eq!(hull, Some(([0.0, 0.0, 2.5], [2.0, 4.0, 3.5])));
    assert_eq!(hull_from_bounds(None), None);
    assert_eq!(hull_from_bounds(Some(([0.0; 3], [1.0, 0.0, 1.0]))), None);
    assert_eq!(
        hull_from_bounds(Some(([0.0; 3], [1.0, f32::NAN, 1.0]))),
        None
    );
}

#[test]
fn world_bounds_follow_the_rotation() {
    let upright = Transform::from_translation(Vec3::new(10.0, 0.0, 0.0));
    let b = dyn_ent_world_bounds(&upright, Some(([0.0; 3], [1.0, 2.0, 3.0])), 9.0);
    assert_eq!((b.mid(), b.half()), ([10.0, 0.0, 0.0], [1.0, 2.0, 3.0]));

    let turned = upright.with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let b = dyn_ent_world_bounds(&turned, Some(([0.0; 3], [1.0, 2.0, 3.0])), 9.0);
    let half = b.half();
    assert!((half[0] - 2.0).abs() < 1e-5 && (half[1] - 1.0).abs() < 1e-5);
    assert!((half[2] - 3.0).abs() < 1e-5);
}

#[test]
fn world_bounds_fall_back_to_a_radius_cube() {
    let at = Transform::from_translation(Vec3::new(1.0, 2.0, 3.0));
    let b = dyn_ent_world_bounds(&at, None, 5.0);
    assert_eq!((b.mid(), b.half()), ([1.0, 2.0, 3.0], [5.0; 3]));
    let tiny = dyn_ent_world_bounds(&at, Some(([0.0; 3], [0.0; 3])), 0.25);
    assert_eq!(tiny.half(), [1.0; 3]);
}

#[test]
fn admission_hides_only_when_no_visible_cell_holds_the_piece() {
    let membership = bits_with(1, 4, &[(2, 5)]);
    let vis = |words: &'static [u32], vis_all| AdmittedCellVis {
        words,
        cell_count: 4,
        vis_all,
    };
    // Cell 2 visible: shown. Only cell 0 visible: hidden.
    assert!(!cell_admission_hides(5, &membership, vis(&[0b0100], false)));
    assert!(cell_admission_hides(5, &membership, vis(&[0b0001], false)));
    // A piece in no cell at all is hidden whenever a vis table exists.
    assert!(cell_admission_hides(6, &membership, vis(&[0b1111], false)));
    // Nothing to judge by.
    assert!(!cell_admission_hides(5, &membership, vis(&[0b0001], true)));
    assert!(!cell_admission_hides(5, &membership, vis(&[], false)));
    assert!(!cell_admission_hides(
        5,
        &DynEntCellBits::default(),
        vis(&[1], false)
    ));
}

#[test]
fn primary_light_words_resize_to_zeroes() {
    let mut words = vec![9; 3];
    ensure_primary_light_words(&mut words, 64, 1, 8);
    let wanted = dyn_ent_primary_light_vis_word_count(64, 1, 8);
    assert_eq!(words.len(), wanted);
    assert!(words.iter().all(|w| *w == 0));
    words.iter_mut().for_each(|w| *w = 1);
    ensure_primary_light_words(&mut words, 64, 1, 8);
    assert!(words.iter().all(|w| *w == 1));
}
