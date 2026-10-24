/// `v` scaled to length one, or straight up when it has no length.
pub fn normalize_or_up(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|part| part * part).sum::<f32>().sqrt();
    if length == 0.0 {
        [0.0, 0.0, 1.0]
    } else {
        v.map(|part| part / length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_is_scaled_to_length_one() {
        assert_eq!(normalize_or_up([0.0, 3.0, 4.0]), [0.0, 0.6, 0.8]);
    }

    #[test]
    fn a_zero_vector_points_up() {
        assert_eq!(normalize_or_up([0.0; 3]), [0.0, 0.0, 1.0]);
    }
}
