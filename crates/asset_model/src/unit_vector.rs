/// `dir` stretched or shrunk to length one; a vector with no length points
/// straight up instead.
pub fn normalised_or_up(dir: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = dir;
    let magnitude = (x * x + y * y + z * z).sqrt();
    if magnitude == 0.0 {
        return [0.0, 0.0, 1.0];
    }
    [x / magnitude, y / magnitude, z / magnitude]
}

/// Older spellings other crates still call; the rename sweep moves them over.
pub fn unit_or_vertical(v: [f32; 3]) -> [f32; 3] {
    normalised_or_up(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_three_four_five_vector_becomes_unit_length() {
        assert_eq!(normalised_or_up([0.0, 3.0, 4.0]), [0.0, 0.6, 0.8]);
    }

    #[test]
    fn no_length_means_up() {
        assert_eq!(normalised_or_up([0.0; 3]), [0.0, 0.0, 1.0]);
    }
}
