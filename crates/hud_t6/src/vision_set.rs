//! BO2 vision-set lerp (cgame cg_visionsets): the lerp styles, one mode's from / to /
//! current sets with its lerp data, the per-field lerps, and how a lerp starts, runs and
//! ends. BO2 has no glow-allowed dvars and no glow fade-out hold, so neither is here.
//! The set payload is the caller's type; it supplies the per-field lerp.
//! Facts: facts/vision-set-t6.md.

/// `visionSetLerpStyle_t` (vision-set-t6.md 1). A mode that has never lerped is `Undefined` and snaps;
/// a finished or snapped lerp is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum VisionSetLerpStyle {
    #[default]
    Undefined = 0,
    None = 1,
    ToLinear = 2,
    ToSmooth = 3,
    BackforthLinear = 4,
    BackforthSmooth = 5,
    CustomValue = 6,
}

impl VisionSetLerpStyle {
    #[must_use]
    pub fn is_backforth(self) -> bool {
        matches!(self, Self::BackforthLinear | Self::BackforthSmooth)
    }
}

/// `visionSetLerpData_t` (vision-set-t6.md 1): times in milliseconds of cg time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisionSetLerpData {
    pub time_start: i32,
    pub time_duration: i32,
    pub lerp_fraction: f32,
    pub style: VisionSetLerpStyle,
}

/// LerpBool (vision-set-t6.md 2). Back-and-forth styles give `from` from the halfway point on; every other style gives `to`.
#[must_use]
pub fn lerp_bool(from: bool, to: bool, fraction: f32, style: VisionSetLerpStyle) -> bool {
    if style.is_backforth() && fraction >= 0.5 {
        from
    } else {
        to
    }
}

/// LerpFloat (vision-set-t6.md 2). TO styles run from -> to; back-and-forth styles reach `to` at half and return to
/// `from`. Smooth styles first ease the fraction by sin(f * pi / 2). CUSTOM_VALUE is linear.
#[must_use]
pub fn lerp_float(from: f32, to: f32, fraction: f32, style: VisionSetLerpStyle) -> f32 {
    use VisionSetLerpStyle as S;
    let fraction = match style {
        S::ToSmooth | S::BackforthSmooth => libm::sinf(fraction * core::f32::consts::PI * 0.5),
        _ => fraction,
    };
    if !style.is_backforth() {
        return from + fraction * (to - from);
    }
    if fraction < 0.5 {
        from + 2.0 * fraction * (to - from)
    } else {
        to + 2.0 * (fraction - 0.5) * (from - to)
    }
}

#[must_use]
pub fn lerp_vec3(
    from: [f32; 3],
    to: [f32; 3],
    fraction: f32,
    style: VisionSetLerpStyle,
) -> [f32; 3] {
    core::array::from_fn(|i| lerp_float(from[i], to[i], fraction, style))
}

#[must_use]
pub fn lerp_vec4(
    from: [f32; 4],
    to: [f32; 4],
    fraction: f32,
    style: VisionSetLerpStyle,
) -> [f32; 4] {
    core::array::from_fn(|i| lerp_float(from[i], to[i], fraction, style))
}

/// BO2's lerp-fraction clamp (vision-set-t6.md 3): a fraction that is not a number counts as 1.
#[must_use]
fn clamp_fraction(f: f32) -> f32 {
    let mut out = 1.0;
    if f - 1.0 < 0.0 {
        out = f;
    }
    if 0.0 <= -f {
        out = 0.0;
    }
    out
}

/// `UpdateVarsLerp` (vision-set-t6.md 3). No change while the style is UNDEFINED or NONE. Once the lerp has
/// run past its end the result becomes `to` (TO styles) or `from` (back-and-forth) and
/// the style becomes NONE; before that every field is lerped at the clamped fraction.
pub fn update_vars_lerp<T: Copy>(
    now: i32,
    from: T,
    to: T,
    lerp: &mut VisionSetLerpData,
    result: &mut T,
    lerp_vars: impl Fn(T, T, f32, VisionSetLerpStyle) -> T,
) {
    if matches!(
        lerp.style,
        VisionSetLerpStyle::Undefined | VisionSetLerpStyle::None
    ) {
        return;
    }
    if lerp.time_start.wrapping_add(lerp.time_duration) < now {
        *result = match lerp.style {
            VisionSetLerpStyle::ToLinear | VisionSetLerpStyle::ToSmooth => to,
            _ => from,
        };
        lerp.style = VisionSetLerpStyle::None;
        return;
    }
    let fraction =
        clamp_fraction(now.wrapping_sub(lerp.time_start) as f32 / lerp.time_duration as f32);
    *result = lerp_vars(from, to, fraction, lerp.style);
}

/// One `visionSetMode_t` slot of cg: `visionSetFrom`, `visionSetTo`, `visionSetCurrent`
/// and `visionSetLerpData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisionSetMode<T> {
    pub from: T,
    pub to: T,
    pub current: T,
    pub lerp: VisionSetLerpData,
}

impl<T: Copy> VisionSetMode<T> {
    /// `CG_VisionSetStartLerp_To` (vision-set-t6.md 4). With a positive duration on a mode that has lerped
    /// before, lerp from the current set to `set`; otherwise snap the current set to
    /// `set` and set the style to NONE.
    pub fn start_lerp_to(&mut self, now: i32, style: VisionSetLerpStyle, set: T, duration_ms: i32) {
        if duration_ms > 0 && self.lerp.style != VisionSetLerpStyle::Undefined {
            self.from = self.current;
            self.to = set;
            self.lerp = VisionSetLerpData {
                time_start: now,
                time_duration: duration_ms,
                lerp_fraction: 0.0,
                style,
            };
        } else {
            self.current = set;
            self.lerp.style = VisionSetLerpStyle::None;
        }
    }

    /// Advance this mode's lerp to `now` (`UpdateVarsLerp` on its sets).
    pub fn update(&mut self, now: i32, lerp_vars: impl Fn(T, T, f32, VisionSetLerpStyle) -> T) {
        update_vars_lerp(
            now,
            self.from,
            self.to,
            &mut self.lerp,
            &mut self.current,
            lerp_vars,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use VisionSetLerpStyle as S;

    #[test]
    fn style_values_match_bo2() {
        assert_eq!(S::Undefined as i32, 0);
        assert_eq!(S::None as i32, 1);
        assert_eq!(S::ToLinear as i32, 2);
        assert_eq!(S::ToSmooth as i32, 3);
        assert_eq!(S::BackforthLinear as i32, 4);
        assert_eq!(S::BackforthSmooth as i32, 5);
        assert_eq!(S::CustomValue as i32, 6);
    }

    #[test]
    fn float_styles() {
        assert_eq!(lerp_float(0.0, 10.0, 0.25, S::ToLinear), 2.5);
        assert_eq!(lerp_float(0.0, 10.0, 0.25, S::CustomValue), 2.5);
        let smooth = lerp_float(0.0, 10.0, 0.5, S::ToSmooth);
        assert!((smooth - 10.0 * libm::sinf(core::f32::consts::PI * 0.25)).abs() < 1e-5);
        assert_eq!(lerp_float(0.0, 10.0, 0.25, S::BackforthLinear), 5.0);
        assert_eq!(lerp_float(0.0, 10.0, 0.5, S::BackforthLinear), 10.0);
        assert_eq!(lerp_float(0.0, 10.0, 1.0, S::BackforthLinear), 0.0);
        assert!(lerp_float(0.0, 10.0, 1.0, S::BackforthSmooth).abs() < 1e-5);
    }

    #[test]
    fn bool_styles() {
        assert!(lerp_bool(false, true, 0.0, S::ToLinear));
        assert!(lerp_bool(false, true, 0.9, S::ToSmooth));
        assert!(lerp_bool(false, true, 0.4, S::BackforthLinear));
        assert!(!lerp_bool(false, true, 0.5, S::BackforthLinear));
        assert!(!lerp_bool(false, true, 0.9, S::BackforthSmooth));
    }

    #[test]
    fn first_start_snaps_then_lerps_then_ends_on_to() {
        let mut mode = VisionSetMode::<f32>::default();
        mode.start_lerp_to(100, S::ToLinear, 4.0, 1000);
        assert_eq!(mode.current, 4.0);
        assert_eq!(mode.lerp.style, S::None);

        mode.start_lerp_to(1000, S::ToLinear, 8.0, 1000);
        assert_eq!(
            (mode.from, mode.to, mode.lerp.style),
            (4.0, 8.0, S::ToLinear)
        );
        mode.update(1500, lerp_float);
        assert_eq!(mode.current, 6.0);
        mode.update(2000, lerp_float);
        assert_eq!(mode.current, 8.0);
        assert_eq!(mode.lerp.style, S::ToLinear);
        mode.update(2001, lerp_float);
        assert_eq!(mode.current, 8.0);
        assert_eq!(mode.lerp.style, S::None);
        mode.update(5000, lerp_float);
        assert_eq!(mode.current, 8.0);
    }

    #[test]
    fn backforth_ends_on_from() {
        let mut mode = VisionSetMode::<f32> {
            current: 1.0,
            lerp: VisionSetLerpData {
                style: S::None,
                ..Default::default()
            },
            ..Default::default()
        };
        mode.start_lerp_to(0, S::BackforthLinear, 3.0, 100);
        mode.update(50, lerp_float);
        assert_eq!(mode.current, 3.0);
        mode.update(101, lerp_float);
        assert_eq!(mode.current, 1.0);
        assert_eq!(mode.lerp.style, S::None);
    }

    #[test]
    fn zero_duration_snaps() {
        let mut mode = VisionSetMode::<f32> {
            current: 1.0,
            lerp: VisionSetLerpData {
                style: S::None,
                ..Default::default()
            },
            ..Default::default()
        };
        mode.start_lerp_to(0, S::ToSmooth, 7.0, 0);
        assert_eq!(mode.current, 7.0);
        assert_eq!(mode.lerp.style, S::None);
    }

    #[test]
    fn nan_fraction_counts_as_one() {
        assert_eq!(clamp_fraction(f32::NAN), 1.0);
        assert_eq!(clamp_fraction(-0.5), 0.0);
        assert_eq!(clamp_fraction(2.0), 1.0);
        assert_eq!(clamp_fraction(0.25), 0.25);
    }
}
