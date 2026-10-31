//! The console variables of the renderer and the client, one table per
//! resource that holds them.

use net::PlayerControlSnapshot;
use render_frontend::adapters::motion::eye_kick::GunOffset;
use render_frontend::assemble::draw_items::dof::{BloomTunables, BokehTunables};
use render_frontend::assemble::draw_items::fog::HazeTunables;
use render_frontend::assemble::draw_items::ColourPaintMethod;
use render_frontend::assemble::draw_items::WarpSettings;
use render_frontend::prepare::scene::eye_parms::GunNearClipSetting;
use render_frontend::prepare::scene::eye_parms::NearClipSetting;
use render_frontend::prepare::scene::eye_parms::SmEnableDvar;
use render_frontend::prepare::scene::eye_parms::SmSunEnableDvar;
use render_frontend::prepare::scene::prop_geom_memo::DetailRampSkinnedTunable;
use render_frontend::prepare::scene::prop_geom_memo::DetailRampTunable;
use render_frontend::prepare::scene::prop_geom_memo::PropCacheEnableTunable;
use render_fx::EffectStainTunables;
use render_fx::MarkSwitch;

use super::Dvar;
use super::DvarSet;
use super::Reading;

// One row per variable: `"name" => field.path`, then the help line. The row
// syntax is not Rust expressions on purpose, so rustfmt keeps each row whole.

/// On/off, kept in a `bool` field.
macro_rules! flag {
    ($name:literal => $($field:ident).+, $about:literal) => {
        Dvar::<Self>::toggle(
            $name,
            $about,
            |owner| owner.$($field).+.into(),
            |owner, value| owner.$($field).+ = value != 0.0,
        )
    };
}

/// A number kept in an `f32` field, refused outside `low..=high`.
macro_rules! ranged {
    ($name:literal => $($field:ident).+ in $low:literal..=$high:literal, $about:literal) => {
        Dvar::<Self>::range(
            $name,
            $about,
            ($low, $high),
            |owner| owner.$($field).+.into(),
            |owner, value| owner.$($field).+ = value,
        )
    };
}

/// On/off for one of the effect-mark switches.
macro_rules! mark_switch {
    ($name:literal => $switch:ident, $about:literal) => {
        Dvar::<Self>::toggle(
            $name,
            $about,
            |marks| marks.is_on(MarkSwitch::$switch).into(),
            |marks, value| marks.set(MarkSwitch::$switch, value != 0.0),
        )
    };
}

impl DvarSet for BloomTunables {
    const DVARS: &'static [Dvar<Self>] = &[
        flag!("r_glow" => enable, "glow on or off"),
        flag!("r_glowUseTweaks" => use_trims, "use the tweak values"),
        flag!("r_glowTweakEnable" => trim_enable, "tweak values on or off"),
        ranged!("r_glowTweakRadius0" => trim_radius in 0.0..=32.0, "glow blur radius"),
        ranged!("r_glowTweakBloomIntensity0" => trim_intensity in 0.0..=20.0, "bloom intensity"),
        ranged!("r_glowTweakBloomCutoff" => trim_cutoff in 0.0..=1.0, "bloom cutoff"),
        ranged!("r_glowTweakBloomDesaturation" => trim_desaturation in 0.0..=1.0, "bloom desaturation"),
    ];
}

impl DvarSet for BokehTunables {
    const DVARS: &'static [Dvar<Self>] = &[
        flag!("r_dof_enable" => enable, "depth of field on or off"),
        flag!("r_dof_tweak" => tweak, "use the tweak values"),
        ranged!("r_dof_nearBlur" => values.close_soften in 4.0..=10.0, "near blur strength"),
        ranged!("r_dof_farBlur" => values.distant_soften in 0.0..=10.0, "far blur strength"),
        ranged!("r_dof_viewModelStart" => values.hand_model_start in 0.0..=128.0, "view model focus start"),
        ranged!("r_dof_viewModelEnd" => values.hand_model_end in 0.0..=128.0, "view model focus end"),
        ranged!("r_dof_nearStart" => values.close_begin in 0.0..=1000.0, "near blur start"),
        ranged!("r_dof_nearEnd" => values.close_stop in 0.0..=1000.0, "near blur end"),
        ranged!("r_dof_farStart" => values.distant_begin in 0.0..=80000.0, "far blur start"),
        ranged!("r_dof_farEnd" => values.distant_stop in 0.0..=80000.0, "far blur end"),
        ranged!("r_dof_bias" => bias in 0.1..=3.0, "blur bias"),
    ];
}

impl DvarSet for HazeTunables {
    const DVARS: &'static [Dvar<Self>] = &[
        flag!("r_fog" => enabled, "fog on or off"),
        Dvar::<Self>::at_least(
            "r_zfar",
            "far clip distance; 0 leaves it to the map",
            0.0,
            |fog| fog.far_clip.into(),
            |fog, value| fog.far_clip = value,
        ),
    ];
}

impl DvarSet for WarpSettings {
    const DVARS: &'static [Dvar<Self>] =
        &[flag!("r_distortion" => enabled, "scene distortion on or off (default 1)")];
}

impl DvarSet for EffectStainTunables {
    const DVARS: &'static [Dvar<Self>] = &[
        mark_switch!("fx_marks" => Marks, "ImpactMark outer gate")
            .noting(|_: &EffectStainTunables| "(alloc + world verts; dyn packed fill)".to_owned()),
        mark_switch!("fx_marks_smodels" => StaticModels, "models Generate OR with _ents"),
        mark_switch!("fx_marks_ents" => Entities, "models Generate OR only; AddEntity* typed gap")
            .noting(|_: &EffectStainTunables| "(AddEntity* typed gap; models Generate OR only)".to_owned()),
        mark_switch!("fx_mark_profile" => Profile, "marks census log/dump, not overlay UI")
            .noting(|_: &EffectStainTunables| "(census log/dump, not overlay UI)".to_owned()),
    ];
}

impl DvarSet for ColourPaintMethod {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "r_fullbright",
        "colour draw method: standard (0, tech 9) or fullbright (1, tech 4)",
        |method| (*method != ColourPaintMethod::Standard).into(),
        |method, value| {
            *method = if value != 0.0 {
                ColourPaintMethod::Unshaded
            } else {
                ColourPaintMethod::Standard
            };
        },
    )
    .noting(|method: &ColourPaintMethod| {
        String::from(match method {
            ColourPaintMethod::Standard => "(standard tech 9)",
            ColourPaintMethod::Unshaded => "(fullbright tech 4)",
            ColourPaintMethod::MaterialProbe => "(debug-material tech 46)",
        })
    })];
}

impl DvarSet for GunOffset {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::free(
            "cg_gun_x",
            "viewmodel offset along view forward",
            "inches",
            |gun| gun.x.into(),
            |gun, value| gun.x = value,
        ),
        Dvar::<Self>::free(
            "cg_gun_y",
            "viewmodel offset along view right",
            "inches",
            |gun| gun.y.into(),
            |gun, value| gun.y = value,
        ),
        Dvar::<Self>::free(
            "cg_gun_z",
            "viewmodel offset along view up",
            "inches",
            |gun| gun.z.into(),
            |gun, value| gun.z = value,
        ),
    ];
}

impl DvarSet for PlayerControlSnapshot {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::free(
        "cl_yawspeed",
        "keyboard +left/+right rate; default 140",
        "deg/s",
        |input| input.key_yaw_speed.into(),
        |input, value| input.key_yaw_speed = value,
    )];
}

impl DvarSet for SmEnableDvar {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "sm_enable",
        "shadow-map gate",
        |gate| gate.enabled.into(),
        |gate, value| gate.enabled = Some(value != 0.0),
    )];
}

impl DvarSet for SmSunEnableDvar {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "sm_sunEnable",
        "used bits 1..=sun",
        |gate| gate.enabled.into(),
        |gate, value| gate.enabled = Some(value != 0.0),
    )];
}

impl DvarSet for PropCacheEnableTunable {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "r_smc_enable",
        "static model cache",
        |gate| gate.enabled.into(),
        |gate, value| gate.enabled = Some(value != 0.0),
    )];
}

impl DvarSet for NearClipSetting {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::free(
        "r_znear",
        "DPVS infinite near; default 4",
        "f",
        |znear| znear.value.into(),
        |znear, value| znear.value = value,
    )];
}

impl DvarSet for GunNearClipSetting {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::free(
        "r_znear_depthhack",
        "viewmodel reverse-Z near; default 0.1",
        "f",
        |znear| znear.value.into(),
        |znear, value| znear.value = value,
    )];
}

/// What a LOD ramp shows besides the variable asked about: the other side of
/// the ramp, what the last frame used, and the values the T5 curve applies.
struct RampNote<'a> {
    other_name: &'a str,
    other: Option<f32>,
    last_frame_scale: Option<f32>,
    last_frame_unit: Option<f32>,
    scale: f32,
    bias: f32,
    applied: f32,
}

fn ramp_note(note: RampNote<'_>) -> String {
    let RampNote { other_name, other, last_frame_scale, last_frame_unit, scale, bias, applied } = note;
    format!(
        "{other_name} = {}  scale_last = {}  world_unit = {}  t5 scale/bias/applied = {scale}/{bias}/{applied}",
        Reading::from(other),
        Reading::from(last_frame_scale),
        Reading::from(last_frame_unit),
    )
}

impl DvarSet for DetailRampTunable {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::free(
            "r_lodScaleRigid",
            "rigid LOD distance scale",
            "f",
            |ramp| ramp.mid_scale.into(),
            |ramp, value| {
                ramp.mid_scale = Some(value);
                ramp.classic_scale = value;
            },
        )
        .noting(|ramp: &DetailRampTunable| {
            ramp_note(RampNote {
                other_name: "r_lodBiasRigid",
                other: ramp.mid_bias,
                last_frame_scale: ramp.ramp_end_scale,
                last_frame_unit: ramp.unit_len,
                scale: ramp.classic_scale,
                bias: ramp.classic_bias,
                applied: ramp.t5.used_reciprocal_factor,
            })
        }),
        Dvar::<Self>::free(
            "r_lodBiasRigid",
            "rigid LOD distance bias",
            "f",
            |ramp| ramp.mid_bias.into(),
            |ramp, value| {
                ramp.mid_bias = Some(value);
                ramp.classic_bias = value;
            },
        )
        .noting(|ramp: &DetailRampTunable| {
            ramp_note(RampNote {
                other_name: "r_lodScaleRigid",
                other: ramp.mid_scale,
                last_frame_scale: ramp.ramp_end_scale,
                last_frame_unit: ramp.unit_len,
                scale: ramp.classic_scale,
                bias: ramp.classic_bias,
                applied: ramp.t5.used_reciprocal_factor,
            })
        }),
    ];
}

impl DvarSet for DetailRampSkinnedTunable {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::free(
            "r_lodScaleSkinned",
            "skinned LOD distance scale",
            "f",
            |ramp| ramp.mid_scale.into(),
            |ramp, value| {
                ramp.mid_scale = Some(value);
                ramp.classic_scale = value;
            },
        )
        .noting(|ramp: &DetailRampSkinnedTunable| {
            ramp_note(RampNote {
                other_name: "r_lodBiasSkinned",
                other: ramp.mid_bias,
                last_frame_scale: ramp.prior_size,
                last_frame_unit: ramp.map_unit,
                scale: ramp.classic_scale,
                bias: ramp.classic_bias,
                applied: ramp.t5.used_reciprocal_factor,
            })
        }),
        Dvar::<Self>::free(
            "r_lodBiasSkinned",
            "skinned LOD distance bias",
            "f",
            |ramp| ramp.mid_bias.into(),
            |ramp, value| {
                ramp.mid_bias = Some(value);
                ramp.classic_bias = value;
            },
        )
        .noting(|ramp: &DetailRampSkinnedTunable| {
            ramp_note(RampNote {
                other_name: "r_lodScaleSkinned",
                other: ramp.mid_scale,
                last_frame_scale: ramp.prior_size,
                last_frame_unit: ramp.map_unit,
                scale: ramp.classic_scale,
                bias: ramp.classic_bias,
                applied: ramp.t5.used_reciprocal_factor,
            })
        }),
    ];
}

#[cfg(test)]
mod tests {
    use crate::dvars::sets::*;
    use crate::dvars::Kind;

    /// A value each kind takes that is not what the resource starts with.
    fn probe(kind: Kind) -> f32 {
        match kind {
            Kind::Toggle => 1.0,
            Kind::Range(_, high) => high,
            Kind::AtLeast(low) => low + 1.0,
            Kind::Free(_) => 2.5,
        }
    }

    fn set_by_name<R: DvarSet>(resource: &mut R, name: &str, text: &str) -> String {
        let dvar = R::DVARS.iter().find(|dvar| dvar.dvar_name == name).unwrap();
        let mut slot = resource;
        dvar.answer(&[text.to_owned()], &mut slot)
    }

    fn read_by_name<R: DvarSet>(resource: &R, name: &str) -> Reading {
        let dvar = R::DVARS.iter().find(|dvar| dvar.dvar_name == name).unwrap();
        (dvar.get)(resource)
    }

    fn reads_back_what_was_set<R: DvarSet + Default>() {
        for dvar in R::DVARS {
            let mut resource = R::default();
            let value = probe(dvar.kind);
            set_by_name(&mut resource, dvar.dvar_name, &value.to_string());
            let expected = Reading::Value(value);
            assert_eq!(read_by_name(&resource, dvar.dvar_name), expected, "{}", dvar.dvar_name);
        }
    }

    #[test]
    fn every_variable_reads_back_what_it_was_set_to() {
        reads_back_what_was_set::<BloomTunables>();
        reads_back_what_was_set::<BokehTunables>();
        reads_back_what_was_set::<HazeTunables>();
        reads_back_what_was_set::<WarpSettings>();
        reads_back_what_was_set::<ColourPaintMethod>();
        reads_back_what_was_set::<GunOffset>();
        reads_back_what_was_set::<PlayerControlSnapshot>();
        reads_back_what_was_set::<SmEnableDvar>();
        reads_back_what_was_set::<SmSunEnableDvar>();
        reads_back_what_was_set::<PropCacheEnableTunable>();
        reads_back_what_was_set::<NearClipSetting>();
        reads_back_what_was_set::<GunNearClipSetting>();
        reads_back_what_was_set::<EffectStainTunables>();
        reads_back_what_was_set::<DetailRampTunable>();
        reads_back_what_was_set::<DetailRampSkinnedTunable>();
    }

    #[test]
    fn depth_of_field_keeps_to_its_domains() {
        let mut dof = BokehTunables::default();
        let near = "r_dof_nearBlur";
        let expected = "usage: r_dof_nearBlur [4..10]";
        assert_eq!(set_by_name(&mut dof, near, "3.9"), expected);
        let expected = "r_dof_nearBlur = 4";
        assert_eq!(set_by_name(&mut dof, near, "4"), expected);
        let expected = "usage: r_dof_bias [0.1..3]";
        let bias = "r_dof_bias";
        assert_eq!(set_by_name(&mut dof, bias, "3.5"), expected);
        assert_eq!(dof.values.close_soften, 4.0);
    }

    #[test]
    fn the_gun_moves_along_one_axis_at_a_time() {
        let mut gun = GunOffset::default();
        set_by_name(&mut gun, "cg_gun_y", "-1.5");
        assert_eq!((gun.x, gun.y, gun.z), (0.0, -1.5, 0.0));
    }

    #[test]
    fn a_lod_scale_or_bias_sets_the_t5_curve_with_it() {
        let mut ramp = DetailRampTunable::default();
        set_by_name(&mut ramp, "r_lodScaleRigid", "1.5");
        set_by_name(&mut ramp, "r_lodBiasRigid", "-2");
        assert_eq!((ramp.mid_scale, ramp.mid_bias), (Some(1.5), Some(-2.0)));
        assert_eq!((ramp.classic_scale, ramp.classic_bias), (1.5, -2.0));

        let mut skinned = DetailRampSkinnedTunable::default();
        set_by_name(&mut skinned, "r_lodScaleSkinned", "0.5");
        assert_eq!((skinned.mid_scale, skinned.classic_scale), (Some(0.5), 0.5));
    }

    #[test]
    fn a_lod_answer_shows_the_other_half_of_the_ramp() {
        let mut ramp = DetailRampTunable::default();
        let said = set_by_name(&mut ramp, "r_lodScaleRigid", "1.5");
        assert!(
            said.starts_with("r_lodScaleRigid = 1.5 r_lodBiasRigid = "),
            "{said}"
        );
        assert!(said.contains("scale_last = unread"), "{said}");
        assert!(said.contains("t5 scale/bias/applied = 1.5/"), "{said}");
    }

    #[test]
    fn fullbright_says_which_technique_it_picked() {
        let mut method = ColourPaintMethod::Standard;
        let expected = "r_fullbright = 1 (fullbright tech 4)";
        assert_eq!(set_by_name(&mut method, "r_fullbright", "1"), expected);
        assert_eq!(method, ColourPaintMethod::Unshaded);
        let expected = "r_fullbright = 0 (standard tech 9)";
        assert_eq!(set_by_name(&mut method, "r_fullbright", "off"), expected);
        method = ColourPaintMethod::MaterialProbe;
        let expected = Reading::Value(1.0);
        assert_eq!(read_by_name(&method, "r_fullbright"), expected, "debug material is not standard");
    }

    #[test]
    fn a_gate_nobody_read_yet_says_so_until_it_is_set() {
        let mut gate = SmEnableDvar::default();
        assert_eq!(read_by_name(&gate, "sm_enable"), Reading::Unread);
        assert_eq!(set_by_name(&mut gate, "sm_enable", "0"), "sm_enable = 0");
        assert!(gate.enabled == Some(false));
    }
}
