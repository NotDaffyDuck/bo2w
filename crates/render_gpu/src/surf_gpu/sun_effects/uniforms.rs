use render_frame::SunEffectsFrame;

/// Byte size of [`SunUniforms`]; the bind group layout and the buffer both use it.
pub(super) const UNIFORM_BYTES: u64 = std::mem::size_of::<SunUniforms>() as u64;

/// Five vec4 rows, matching `SunUniforms` in `sun.wgsl` row for row.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct SunUniforms {
    sun_clip: [f32; 4],
    screen: [f32; 4],
    flare_fade: [f32; 4],
    blind_fade: [f32; 4],
    glare_fade: [f32; 4],
}

fn flag(on: bool) -> f32 {
    f32::from(u8::from(on))
}

impl SunUniforms {
    pub(super) fn from_frame(f: &SunEffectsFrame) -> Self {
        let ms = |v: i32| v as f32;
        Self {
            sun_clip: f.to_clip,
            screen: [f.viewport_w, f.viewport_h, f.sprite_size, f.flare_size],
            flare_fade: [
                f.flare_alpha,
                ms(f.dt_ms),
                ms(f.flare_fade_in_ms),
                ms(f.flare_fade_out_ms),
            ],
            blind_fade: [
                f.blind_goal,
                ms(f.blind_fade_in_ms),
                ms(f.blind_fade_out_ms),
                f.glare_goal,
            ],
            glare_fade: [
                ms(f.glare_fade_in_ms),
                ms(f.glare_fade_out_ms),
                flag(f.camera_cut),
                flag(f.behind_camera),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_block_is_five_rows() {
        assert_eq!(UNIFORM_BYTES, 80);
    }
}
