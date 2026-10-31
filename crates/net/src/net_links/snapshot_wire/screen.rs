//! What a player's own screen needs on top of the world: script-made HUD elements, the shock
//! effect, camera tints and blur, menu commands, the map-pick cursor and the weapon-view inputs.

use hud_t6::flashbang::{
    ShellshockLookControl, ShellshockParms, ShellshockScreenBlend, ShellshockSound, ShellshockView,
    ShellshockVisionSet, ShockViewType,
};
use hud_t6::overlay_piece::{OVERLAY_PIECE_LIMIT, OverlayPiece};
use sim::{
    LocationSelection, MenuOrder, MenuCommandKind, ScriptBlur, ScriptDepthOfField, T6ViewInputs,
    ViewEffects, VisionChange,
};

use super::codec::{Wire, refuse, take_counted};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

wire_fields!(OverlayPiece {
    piece_kind,
    y,
    x,
    z,
    aim_ent,
    text_size,
    text_size_was,
    text_grow_from,
    text_grow_ms,
    caption,
    font,
    anchor,
    screen_anchor,
    tint_rgba,
    tint_was,
    fade_from,
    fade_len,
    height,
    width,
    skin_no,
    offscreen_material_index,
    was_y,
    was_x,
    anchor_was,
    screen_anchor_was,
    slide_from,
    slide_ms,
    was_tall,
    was_wide,
    grow_from,
    grow_ms,
    val,
    time,
    lasts,
    words,
    sort,
    glow_rgba,
    fx_born,
    fx_char_ms,
    fx_fade_from,
    fx_fade_len,
    fx_redact_decay_start_time,
    fx_redact_decay_duration,
    sound_no,
    bits,
    ui3d_window,
});

/// One bank of script HUD elements; a bank never holds more than the game's slot count.
pub(super) fn put_hud_bank(wr: &mut ByteSinkWriter, bank: &[OverlayPiece]) {
    let kept = &bank[..bank.len().min(OVERLAY_PIECE_LIMIT)];
    wr.put_u8(kept.len() as u8);
    for elem in kept {
        elem.put(wr);
    }
}

pub(super) fn take_hud_bank(rd: &mut ByteCursorReader<'_>) -> Result<Vec<OverlayPiece>, PacketDecodeFault> {
    let count = usize::from(rd.get_u8()?);
    if count > OVERLAY_PIECE_LIMIT {
        return refuse("hud bank longer than 31");
    }
    take_counted(rd, count, OVERLAY_PIECE_LIMIT)
}

impl Wire for ShockViewType {
    fn put(&self, wr: &mut ByteSinkWriter) {
        (*self as i32).put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match ShockViewType::from_i32(i32::take(rd)?) {
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

wire_fields!(ShellshockView { fade_for_ms, kick_rate, kick_radius });

wire_fields!(ShellshockSound {
    touches,
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

wire_fields!(ShellshockLookControl { touches, fade_for_ms, touchiness, pitch_cap, yaw_cap });

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
pub(super) fn put_shellshock(wr: &mut ByteSinkWriter, shock: &Option<ShellshockParms>) {
    super::codec::put_flagged(wr, shock);
}

pub(super) fn take_shellshock(rd: &mut ByteCursorReader<'_>) -> Result<Option<ShellshockParms>, PacketDecodeFault> {
    match rd.get_u8()? {
        0 => Ok(None),
        _ => ShellshockParms::take(rd).map(Some),
    }
}

wire_fields!(VisionChange { name, lasts_ms, set_ms });

wire_fields!(ScriptBlur { from, to, set_ms, lasts_ms });

wire_fields!(ScriptDepthOfField { near_from, near_to, far_from, far_to, near_smear, far_smear });

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
    fn put(&self, wr: &mut ByteSinkWriter) {
        match self {
            Self::Show(menu) => {
                wr.put_u8(0);
                menu.put(wr);
            }
            Self::ClosePopup => wr.put_u8(1),
            Self::CloseInGame => wr.put_u8(2),
        }
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match rd.get_u8()? {
            0 => String::take(rd).map(Self::Show),
            1 => Ok(Self::ClosePopup),
            2 => Ok(Self::CloseInGame),
            _ => refuse("bad menu command tag"),
        }
    }
}

wire_fields!(MenuOrder { serial, kind });

wire_fields!(LocationSelection { material, choose_direction, reach });

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
