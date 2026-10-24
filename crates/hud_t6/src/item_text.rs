//! Where an item's text sits in its rect: text align mode bits 0..1 place it across (0 left,
//! 1 centre, 2 right), bits 2..3 down (0 top, 4 by its own height, 8 middle, else bottom).
//!
//! TODO(fact): BO2's item text align modes are not in the facts yet (QUESTIONS.md 10-21 Clean1
//! hud item text); until answered these are MW2's.

/// The text's (x, y) in the rect, given its measured size.
#[must_use]
pub fn item_text_origin(
    rect_x: f32,
    rect_y: f32,
    rect_w: f32,
    rect_h: f32,
    text_align_mode: i32,
    text_align_x: f32,
    text_align_y: f32,
    measured_w: f32,
    measured_h: f32,
) -> (f32, f32) {
    let mut x = text_align_x;
    let h_mode = text_align_mode & 3;
    if h_mode != 0 {
        let mut d = rect_w - measured_w;
        if h_mode == 1 {
            d *= 0.5;
        }
        x += d;
    }
    x += rect_x;
    let y =
        item_get_text_placement_y(text_align_mode & 0xc, text_align_y, rect_h, measured_h) + rect_y;
    (x, y)
}

#[must_use]
pub fn item_get_text_placement_y(align_y: i32, y0: f32, container_h: f32, self_h: f32) -> f32 {
    match align_y {
        0 => y0,
        4 => y0 + self_h,
        8 => y0 + (container_h + self_h) * 0.5,
        _ => y0 + container_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centred_text_sits_in_the_middle() {
        assert_eq!(item_text_origin(0.0, 0.0, 100.0, 20.0, 1 | 8, 0.0, 0.0, 40.0, 10.0), (30.0, 15.0));
        assert_eq!(item_text_origin(5.0, 5.0, 100.0, 20.0, 2, 1.0, 2.0, 40.0, 10.0), (66.0, 7.0));
        assert_eq!(item_get_text_placement_y(12, 0.0, 20.0, 10.0), 20.0);
    }
}
