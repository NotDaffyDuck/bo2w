//! What a player's own screen needs on top of the world: script-made HUD elements, the shock
//! effect, camera tints and blur, menu commands, the map-pick cursor and the weapon-view inputs.

use hud_t6::flashbang::{
    ShellshockLookControl, ShellshockParms, ShellshockScreenBlend, ShellshockSound, ShellshockView,
    ShellshockVisionSet, ShockViewType,
};
use hud_t6::hudelem::{HUDELEM_BANK_CAPACITY, HudElem};
use sim::{
    LocationSelection, MenuCommand, MenuCommandKind, ScriptBlur, ScriptDepthOfField, T6ViewInputs,
    ViewEffects, VisionChange,
};

use super::codec::{Decoded, Wire, refuse, take_counted};
use crate::transport::wire::{WireReader, WireWriter};

wire_fields!(HudElem {
    elem_type,
    y,
    x,
    z,
    target_ent_num,
    font_scale,
    from_font_scale,
    font_scale_start_time,
    font_scale_time,
    label,
    font,
    align_org,
    align_screen,
    color_rgba,
    from_color_rgba,
    fade_start_time,
    fade_time,
    height,
    width,
    material_index,
    offscreen_material_index,
    from_y,
    from_x,
    from_align_org,
    from_align_screen,
    move_start_time,
    move_time,
    from_height,
    from_width,
    scale_start_time,
    scale_time,
    value,
    time,
    duration,
    text,
    sort,
    glow_color_rgba,
    fx_birth_time,
    fx_letter_time,
    fx_decay_start_time,
    fx_decay_duration,
    fx_redact_decay_start_time,
    fx_redact_decay_duration,
    sound_id,
    flags,
    ui3d_window,
});

/// One bank of script HUD elements; a bank never holds more than the game's slot count.
pub(super) fn put_hud_bank(out: &mut WireWriter, bank: &[HudElem]) {
    let kept = &bank[..bank.len().min(HUDELEM_BANK_CAPACITY)];
    out.put_u8(kept.len() as u8);
    for elem in kept {
        elem.put(out);
    }
}

pub(super) fn take_hud_bank(input: &mut WireReader<'_>) -> Decoded<Vec<HudElem>> {
    let count = usize::from(input.get_u8()?);
    if count > HUDELEM_BANK_CAPACITY {
        return refuse("hud bank longer than 31");
    }
    take_counted(input, count, HUDELEM_BANK_CAPACITY)
}

impl Wire for ShockViewType {
    fn put(&self, out: &mut WireWriter) {
        (*self as i32).put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match ShockViewType::from_i32(i32::take(input)?) {
            Some(kind) => Ok(kind),
            None => refuse("bad shock screen type"),
        }
    }
}

wire_fields!(ShellshockScreenBlend {
    blurred_fade_ms,
    blurred_effect_ms,
    flash_white_fade_ms,
    flash_shot_fade_ms,
    kind,
});

wire_fields!(ShellshockView { fade_ms, kick_rate, kick_radius });

wire_fields!(ShellshockSound {
    affect,
    loop_alias,
    loop_silent_alias,
    end_alias,
    end_abort_alias,
    fade_in_ms,
    fade_out_ms,
    loop_fade_ms,
    loop_end_delay_ms,
    room_type,
    dry_level,
    wet_level,
    mod_end_delay_ms,
    snapshot,
});

wire_fields!(ShellshockLookControl { affect, fade_ms, sensitivity, max_pitch_speed, max_yaw_speed });

wire_fields!(ShellshockVisionSet { name, in_ms, out_ms });

wire_fields!(ShellshockParms {
    screen_blend,
    view,
    sound,
    look_control,
    movement_scale,
    animation,
    vision_set,
});

/// The shock effect is present for any non-zero lead byte, not only for a 1.
pub(super) fn put_shellshock(out: &mut WireWriter, shock: &Option<ShellshockParms>) {
    super::codec::put_flagged(out, shock);
}

pub(super) fn take_shellshock(input: &mut WireReader<'_>) -> Decoded<Option<ShellshockParms>> {
    match input.get_u8()? {
        0 => Ok(None),
        _ => ShellshockParms::take(input).map(Some),
    }
}

wire_fields!(VisionChange { name, duration_ms, set_ms });

wire_fields!(ScriptBlur { from, to, set_ms, duration_ms });

wire_fields!(ScriptDepthOfField { near_start, near_end, far_start, far_end, near_blur, far_blur });

wire_fields!(ViewEffects {
    naked_vision: flag "bad vision tag",
    thermal_vision: flag "bad vision tag",
    missile_vision: flag "bad vision tag",
    night_vision: flag "bad vision tag",
    pain_vision: flag "bad vision tag",
    blur: flag "bad blur tag",
    depth_of_field,
});

impl Wire for MenuCommandKind {
    fn put(&self, out: &mut WireWriter) {
        match self {
            Self::Open(menu) => {
                out.put_u8(0);
                menu.put(out);
            }
            Self::ClosePopup => out.put_u8(1),
            Self::CloseInGame => out.put_u8(2),
        }
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match input.get_u8()? {
            0 => String::take(input).map(Self::Open),
            1 => Ok(Self::ClosePopup),
            2 => Ok(Self::CloseInGame),
            _ => refuse("bad menu command tag"),
        }
    }
}

wire_fields!(MenuCommand { serial, kind });

wire_fields!(LocationSelection { material, choose_direction, radius });

wire_fields!(T6ViewInputs {
    weap_anim,
    weap_anim_left,
    weaponstate,
    weaponstate_left,
    weapon_time,
    weapon_time_left,
    intro_shots_fired,
    ever_had,
    needs_rechamber,
});
