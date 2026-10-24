//! Reading a light probe's ambient colour out of the four bytes it is stored
//! in.

/// 2/255: a colour byte spans 0 to 2 before it is squared.
const COLOR_STEP: f32 = f32::from_bits(0x3c00_8081);
/// 1/1020: the alpha byte spans 0 to a quarter.
const ALPHA_STEP: f32 = f32::from_bits(0x3a80_8081);

/// Unpacks `[r, g, b, a]` bytes. The colour channels are stored as square
/// roots, so each is squared on the way out; alpha is a plain scale.
pub fn light_probe_ambient_from_packed(packed: [u8; 4]) -> [f32; 4] {
    let channel = |byte: u8| {
        let scaled = COLOR_STEP * f32::from(byte);
        scaled * scaled
    };
    [
        channel(packed[0]),
        channel(packed[1]),
        channel(packed[2]),
        ALPHA_STEP * f32::from(packed[3]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn all_zero_bytes_are_black() {
        assert_eq!(light_probe_ambient_from_packed([0; 4]), [0.0; 4]);
    }

    #[test]
    fn a_full_channel_squares_up_to_four() {
        let ambient = light_probe_ambient_from_packed([255, 0, 0, 255]);
        assert!(near(ambient[0], 4.0));
        assert_eq!(ambient[1], 0.0);
        assert_eq!(ambient[2], 0.0);
    }

    #[test]
    fn colour_is_squared_but_alpha_is_not() {
        let ambient = light_probe_ambient_from_packed([51, 102, 153, 102]);
        assert!(near(ambient[0], 0.16));
        assert!(near(ambient[1], 0.64));
        assert!(near(ambient[2], 1.44));
        assert!(near(ambient[3], 0.1));
    }

    #[test]
    fn full_alpha_is_a_quarter() {
        let ambient = light_probe_ambient_from_packed([0, 0, 0, 255]);
        assert!(near(ambient[3], 0.25));
    }
}
