//! The console variables of the renderer and the client, one table per
//! resource that holds them.

use net::ClientActionInput;
use render_frontend::{
    adapters::anim::view_kick::GunOffset,
    assemble::drawsurf::{
        ColourDrawMethod, DistortionSettings,
        dof::{DofDvars, GlowDvars},
        fog::FogDvars,
    },
    prepare::scene::{
        smodel_geom_cache::{LodRampDvar, LodRampSkinnedDvar, SmcEnableDvar},
        view_parms::{RZnearDepthhackDvar, RZnearDvar, SmEnableDvar, SmSunEnableDvar},
    },
};
use render_fx::FxMarkDvars;

use super::{Dvar, DvarSet, Reading};

impl DvarSet for GlowDvars {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::toggle(
            "r_glow",
            "glow on or off",
            |glow| glow.enable.into(),
            |glow, value| glow.enable = value != 0.0,
        ),
        Dvar::<Self>::toggle(
            "r_glowUseTweaks",
            "use the tweak values",
            |glow| glow.use_tweaks.into(),
            |glow, value| glow.use_tweaks = value != 0.0,
        ),
        Dvar::<Self>::toggle(
            "r_glowTweakEnable",
            "tweak values on or off",
            |glow| glow.tweak_enable.into(),
            |glow, value| glow.tweak_enable = value != 0.0,
        ),
        Dvar::<Self>::range(
            "r_glowTweakRadius0",
            "glow blur radius",
            (0.0, 32.0),
            |glow| glow.tweak_radius.into(),
            |glow, value| glow.tweak_radius = value,
        ),
        Dvar::<Self>::range(
            "r_glowTweakBloomIntensity0",
            "bloom intensity",
            (0.0, 20.0),
            |glow| glow.tweak_intensity.into(),
            |glow, value| glow.tweak_intensity = value,
        ),
        Dvar::<Self>::range(
            "r_glowTweakBloomCutoff",
            "bloom cutoff",
            (0.0, 1.0),
            |glow| glow.tweak_cutoff.into(),
            |glow, value| glow.tweak_cutoff = value,
        ),
        Dvar::<Self>::range(
            "r_glowTweakBloomDesaturation",
            "bloom desaturation",
            (0.0, 1.0),
            |glow| glow.tweak_desaturation.into(),
            |glow, value| glow.tweak_desaturation = value,
        ),
    ];
}

impl DvarSet for DofDvars {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::toggle(
            "r_dof_enable",
            "depth of field on or off",
            |dof| dof.enable.into(),
            |dof, value| dof.enable = value != 0.0,
        ),
        Dvar::<Self>::toggle(
            "r_dof_tweak",
            "use the tweak values",
            |dof| dof.tweak.into(),
            |dof, value| dof.tweak = value != 0.0,
        ),
        Dvar::<Self>::range(
            "r_dof_nearBlur",
            "near blur strength",
            (4.0, 10.0),
            |dof| dof.values.near_blur.into(),
            |dof, value| dof.values.near_blur = value,
        ),
        Dvar::<Self>::range(
            "r_dof_farBlur",
            "far blur strength",
            (0.0, 10.0),
            |dof| dof.values.far_blur.into(),
            |dof, value| dof.values.far_blur = value,
        ),
        Dvar::<Self>::range(
            "r_dof_viewModelStart",
            "view model focus start",
            (0.0, 128.0),
            |dof| dof.values.view_model_start.into(),
            |dof, value| dof.values.view_model_start = value,
        ),
        Dvar::<Self>::range(
            "r_dof_viewModelEnd",
            "view model focus end",
            (0.0, 128.0),
            |dof| dof.values.view_model_end.into(),
            |dof, value| dof.values.view_model_end = value,
        ),
        Dvar::<Self>::range(
            "r_dof_nearStart",
            "near blur start",
            (0.0, 1000.0),
            |dof| dof.values.near_start.into(),
            |dof, value| dof.values.near_start = value,
        ),
        Dvar::<Self>::range(
            "r_dof_nearEnd",
            "near blur end",
            (0.0, 1000.0),
            |dof| dof.values.near_end.into(),
            |dof, value| dof.values.near_end = value,
        ),
        Dvar::<Self>::range(
            "r_dof_farStart",
            "far blur start",
            (0.0, 80000.0),
            |dof| dof.values.far_start.into(),
            |dof, value| dof.values.far_start = value,
        ),
        Dvar::<Self>::range(
            "r_dof_farEnd",
            "far blur end",
            (0.0, 80000.0),
            |dof| dof.values.far_end.into(),
            |dof, value| dof.values.far_end = value,
        ),
        Dvar::<Self>::range(
            "r_dof_bias",
            "blur bias",
            (0.1, 3.0),
            |dof| dof.bias.into(),
            |dof, value| dof.bias = value,
        ),
    ];
}

impl DvarSet for FogDvars {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::toggle(
            "r_fog",
            "fog on or off",
            |fog| fog.enabled.into(),
            |fog, value| fog.enabled = value != 0.0,
        ),
        Dvar::<Self>::at_least(
            "r_zfar",
            "far clip distance; 0 leaves it to the map",
            0.0,
            |fog| fog.zfar.into(),
            |fog, value| fog.zfar = value,
        ),
    ];
}

impl DvarSet for DistortionSettings {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "r_distortion",
        "scene distortion on or off (default 1)",
        |distortion| distortion.enabled.into(),
        |distortion, value| distortion.enabled = value != 0.0,
    )];
}

impl DvarSet for ColourDrawMethod {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "r_fullbright",
        "colour draw method: standard (0, tech 9) or fullbright (1, tech 4)",
        |method| (*method != ColourDrawMethod::Standard).into(),
        |method, value| {
            *method = if value != 0.0 {
                ColourDrawMethod::Fullbright
            } else {
                ColourDrawMethod::Standard
            };
        },
    )
    .noting(|method: &ColourDrawMethod| {
        match method {
            ColourDrawMethod::Standard => "(standard tech 9)",
            ColourDrawMethod::Fullbright => "(fullbright tech 4)",
            ColourDrawMethod::DebugMaterial => "(debug-material tech 46)",
        }
        .to_owned()
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

impl DvarSet for ClientActionInput {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::free(
        "cl_yawspeed",
        "keyboard +left/+right rate; default 140",
        "deg/s",
        |input| input.cl_yawspeed.into(),
        |input, value| input.cl_yawspeed = value,
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

impl DvarSet for SmcEnableDvar {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::toggle(
        "r_smc_enable",
        "static model cache",
        |gate| gate.enabled.into(),
        |gate, value| gate.enabled = Some(value != 0.0),
    )];
}

impl DvarSet for RZnearDvar {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::free(
        "r_znear",
        "DPVS infinite near; default 4",
        "f",
        |znear| znear.value.into(),
        |znear, value| znear.value = value,
    )];
}

impl DvarSet for RZnearDepthhackDvar {
    const DVARS: &'static [Dvar<Self>] = &[Dvar::<Self>::free(
        "r_znear_depthhack",
        "viewmodel reverse-Z near; default 0.1",
        "f",
        |znear| znear.value.into(),
        |znear, value| znear.value = value,
    )];
}

impl DvarSet for FxMarkDvars {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::toggle(
            "fx_marks",
            "ImpactMark outer gate",
            |marks| marks.fx_marks.into(),
            |marks, value| marks.fx_marks = value != 0.0,
        )
        .noting(|_: &FxMarkDvars| "(alloc + world verts; dyn packed fill)".to_owned()),
        Dvar::<Self>::toggle(
            "fx_marks_smodels",
            "models Generate OR with _ents",
            |marks| marks.fx_marks_smodels.into(),
            |marks, value| marks.fx_marks_smodels = value != 0.0,
        ),
        Dvar::<Self>::toggle(
            "fx_marks_ents",
            "models Generate OR only; AddEntity* typed gap",
            |marks| marks.fx_marks_ents.into(),
            |marks, value| marks.fx_marks_ents = value != 0.0,
        )
        .noting(|_: &FxMarkDvars| "(AddEntity* typed gap; models Generate OR only)".to_owned()),
        Dvar::<Self>::toggle(
            "fx_mark_profile",
            "marks census log/dump, not overlay UI",
            |marks| marks.fx_mark_profile.into(),
            |marks, value| marks.fx_mark_profile = value != 0.0,
        )
        .noting(|_: &FxMarkDvars| "(census log/dump, not overlay UI)".to_owned()),
    ];
}

/// What a LOD ramp shows besides the variable asked about: the other side of
/// the ramp, what the last frame used, and the values the T5 curve applies.
fn ramp_note(
    other: (&str, Option<f32>),
    last: (Option<f32>, Option<f32>),
    t5: (f32, f32, f32),
) -> String {
    let (name, other) = other;
    let (scale_last, world_unit) = last;
    let (scale, bias, applied) = t5;
    format!(
        "{name} = {}  scale_last = {}  world_unit = {}  t5 scale/bias/applied = {scale}/{bias}/{applied}",
        Reading::from(other),
        Reading::from(scale_last),
        Reading::from(world_unit),
    )
}

impl DvarSet for LodRampDvar {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::free(
            "r_lodScaleRigid",
            "rigid LOD distance scale",
            "f",
            |ramp| ramp.scale_mid.into(),
            |ramp, value| {
                ramp.scale_mid = Some(value);
                ramp.t5_scale = value;
            },
        )
        .noting(|ramp: &LodRampDvar| {
            ramp_note(
                ("r_lodBiasRigid", ramp.bias_mid),
                (ramp.scale_last, ramp.world_unit),
                (ramp.t5_scale, ramp.t5_bias, ramp.t5.applied_inv_scale),
            )
        }),
        Dvar::<Self>::free(
            "r_lodBiasRigid",
            "rigid LOD distance bias",
            "f",
            |ramp| ramp.bias_mid.into(),
            |ramp, value| {
                ramp.bias_mid = Some(value);
                ramp.t5_bias = value;
            },
        )
        .noting(|ramp: &LodRampDvar| {
            ramp_note(
                ("r_lodScaleRigid", ramp.scale_mid),
                (ramp.scale_last, ramp.world_unit),
                (ramp.t5_scale, ramp.t5_bias, ramp.t5.applied_inv_scale),
            )
        }),
    ];
}

impl DvarSet for LodRampSkinnedDvar {
    const DVARS: &'static [Dvar<Self>] = &[
        Dvar::<Self>::free(
            "r_lodScaleSkinned",
            "skinned LOD distance scale",
            "f",
            |ramp| ramp.scale_mid.into(),
            |ramp, value| {
                ramp.scale_mid = Some(value);
                ramp.t5_scale = value;
            },
        )
        .noting(|ramp: &LodRampSkinnedDvar| {
            ramp_note(
                ("r_lodBiasSkinned", ramp.bias_mid),
                (ramp.scale_last, ramp.world_unit),
                (ramp.t5_scale, ramp.t5_bias, ramp.t5.applied_inv_scale),
            )
        }),
        Dvar::<Self>::free(
            "r_lodBiasSkinned",
            "skinned LOD distance bias",
            "f",
            |ramp| ramp.bias_mid.into(),
            |ramp, value| {
                ramp.bias_mid = Some(value);
                ramp.t5_bias = value;
            },
        )
        .noting(|ramp: &LodRampSkinnedDvar| {
            ramp_note(
                ("r_lodScaleSkinned", ramp.scale_mid),
                (ramp.scale_last, ramp.world_unit),
                (ramp.t5_scale, ramp.t5_bias, ramp.t5.applied_inv_scale),
            )
        }),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let dvar = R::DVARS.iter().find(|dvar| dvar.name == name).unwrap();
        let mut slot = resource;
        dvar.answer(&[text.to_owned()], &mut slot)
    }

    fn read_by_name<R: DvarSet>(resource: &R, name: &str) -> Reading {
        let dvar = R::DVARS.iter().find(|dvar| dvar.name == name).unwrap();
        (dvar.get)(resource)
    }

    fn reads_back_what_was_set<R: DvarSet + Default>() {
        for dvar in R::DVARS {
            let mut resource = R::default();
            let value = probe(dvar.kind);
            set_by_name(&mut resource, dvar.name, &value.to_string());
            assert_eq!(
                read_by_name(&resource, dvar.name),
                Reading::Value(value),
                "{}",
                dvar.name
            );
        }
    }

    #[test]
    fn every_variable_reads_back_what_it_was_set_to() {
        reads_back_what_was_set::<GlowDvars>();
        reads_back_what_was_set::<DofDvars>();
        reads_back_what_was_set::<FogDvars>();
        reads_back_what_was_set::<DistortionSettings>();
        reads_back_what_was_set::<ColourDrawMethod>();
        reads_back_what_was_set::<GunOffset>();
        reads_back_what_was_set::<ClientActionInput>();
        reads_back_what_was_set::<SmEnableDvar>();
        reads_back_what_was_set::<SmSunEnableDvar>();
        reads_back_what_was_set::<SmcEnableDvar>();
        reads_back_what_was_set::<RZnearDvar>();
        reads_back_what_was_set::<RZnearDepthhackDvar>();
        reads_back_what_was_set::<FxMarkDvars>();
        reads_back_what_was_set::<LodRampDvar>();
        reads_back_what_was_set::<LodRampSkinnedDvar>();
    }

    #[test]
    fn depth_of_field_keeps_to_its_domains() {
        let mut dof = DofDvars::default();
        assert_eq!(
            set_by_name(&mut dof, "r_dof_nearBlur", "3.9"),
            "usage: r_dof_nearBlur [4..10]"
        );
        assert_eq!(
            set_by_name(&mut dof, "r_dof_nearBlur", "4"),
            "r_dof_nearBlur = 4"
        );
        assert_eq!(
            set_by_name(&mut dof, "r_dof_bias", "3.5"),
            "usage: r_dof_bias [0.1..3]"
        );
        assert_eq!(dof.values.near_blur, 4.0);
    }

    #[test]
    fn the_gun_moves_along_one_axis_at_a_time() {
        let mut gun = GunOffset::default();
        set_by_name(&mut gun, "cg_gun_y", "-1.5");
        assert_eq!((gun.x, gun.y, gun.z), (0.0, -1.5, 0.0));
    }

    #[test]
    fn a_lod_scale_or_bias_sets_the_t5_curve_with_it() {
        let mut ramp = LodRampDvar::default();
        set_by_name(&mut ramp, "r_lodScaleRigid", "1.5");
        set_by_name(&mut ramp, "r_lodBiasRigid", "-2");
        assert_eq!((ramp.scale_mid, ramp.bias_mid), (Some(1.5), Some(-2.0)));
        assert_eq!((ramp.t5_scale, ramp.t5_bias), (1.5, -2.0));

        let mut skinned = LodRampSkinnedDvar::default();
        set_by_name(&mut skinned, "r_lodScaleSkinned", "0.5");
        assert_eq!((skinned.scale_mid, skinned.t5_scale), (Some(0.5), 0.5));
    }

    #[test]
    fn a_lod_answer_shows_the_other_half_of_the_ramp() {
        let mut ramp = LodRampDvar::default();
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
        let mut method = ColourDrawMethod::Standard;
        assert_eq!(
            set_by_name(&mut method, "r_fullbright", "1"),
            "r_fullbright = 1 (fullbright tech 4)"
        );
        assert_eq!(method, ColourDrawMethod::Fullbright);
        assert_eq!(
            set_by_name(&mut method, "r_fullbright", "off"),
            "r_fullbright = 0 (standard tech 9)"
        );
        method = ColourDrawMethod::DebugMaterial;
        assert_eq!(
            read_by_name(&method, "r_fullbright"),
            Reading::Value(1.0),
            "debug material is not standard"
        );
    }

    #[test]
    fn a_gate_nobody_read_yet_says_so_until_it_is_set() {
        let mut gate = SmEnableDvar::default();
        assert_eq!(read_by_name(&gate, "sm_enable"), Reading::Unread);
        assert_eq!(set_by_name(&mut gate, "sm_enable", "0"), "sm_enable = 0");
        assert_eq!(gate.enabled, Some(false));
    }
}
