
pub const BUTTON_MELEE: u32 = playerstate_iw4::buttons::MELEE_CHARGE;

pub const PLAYER_MELEE_RANGE_DEFAULT: f32 = 64.0;

pub const PLAYER_MELEE_WIDTH_DEFAULT: f32 = 10.0;
pub const PLAYER_MELEE_HEIGHT_DEFAULT: f32 = 10.0;

pub const MELEE_TRACE_OFFSETS: [[f32; 2]; 9] = [
    [0.0, 0.0],
    [0.5, 0.0],
    [-0.5, 0.0],
    [0.0, 0.5],
    [0.0, -0.5],
    [1.0, 1.0],
    [-1.0, 1.0],
    [1.0, -1.0],
    [-1.0, -1.0],
];

pub fn melee_trace_count(width: f32, height: f32) -> usize {
    if width > 0.0 || height > 0.0 {
        MELEE_TRACE_OFFSETS.len()
    } else {
        1
    }
}

pub fn melee_trace_end(
    muzzle: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    range: f32,
    width: f32,
    height: f32,
    offset: [f32; 2],
) -> [f32; 3] {
    let wr = width * offset[0];
    let hu = height * offset[1];
    [
        muzzle[0] + forward[0] * range + right[0] * wr + up[0] * hu,
        muzzle[1] + forward[1] * range + right[1] * wr + up[1] * hu,
        muzzle[2] + forward[2] * range + right[2] * wr + up[2] * hu,
    ]
}
