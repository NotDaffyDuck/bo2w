//! Turning one operand or one component of an operation into a WGSL
//! expression.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Sm3WgslError, component_name, register_name, texture_array_name};
use crate::abi::SamplerTextureDimension;
use crate::ir::{
    Sm3Opcode, Sm3Register, Sm3RegisterFile, Sm3Source, Sm3SourceModifier, TEXLD_PROJECT,
};

/// What an expression may read at one instruction.
pub(super) struct Scope<'a> {
    pub at_word: usize,
    /// Registers fed in from outside, always readable.
    pub external: &'a BTreeSet<Sm3Register>,
    /// Components of local registers written so far.
    pub defined: &'a BTreeMap<Sm3Register, u8>,
    pub samplers: &'a BTreeMap<u16, SamplerTextureDimension>,
    /// Name of the helper that loads a constant by runtime index, when the
    /// stage has one.
    pub relative_helper: Option<&'a str>,
}

impl Scope<'_> {
    /// One component of a source operand, modifier applied.
    pub fn source(&self, source: &Sm3Source, lane: usize) -> Result<String, Sm3WgslError> {
        if source.register.file == Sm3RegisterFile::Sampler {
            return Err(Sm3WgslError::InvalidSamplerOperand {
                at_word: self.at_word,
            });
        }
        let base = self.read(source, source.swizzle[lane])?;
        Ok(match source.modifier {
            Sm3SourceModifier::None | Sm3SourceModifier::Unknown(_) => base,
            Sm3SourceModifier::Negate => format!("(-{base})"),
            Sm3SourceModifier::Bias => format!("({base} - 0.5)"),
            Sm3SourceModifier::BiasNegate => format!("(0.5 - {base})"),
            Sm3SourceModifier::Sign => format!("({base} * 2.0 - 1.0)"),
            Sm3SourceModifier::SignNegate => format!("(1.0 - {base} * 2.0)"),
            Sm3SourceModifier::Complement => format!("(1.0 - {base})"),
            Sm3SourceModifier::Double => format!("({base} * 2.0)"),
            Sm3SourceModifier::DoubleNegate => format!("(-{base} * 2.0)"),
            Sm3SourceModifier::DivideByZ | Sm3SourceModifier::DivideByW => {
                let divisor_lane = if source.modifier == Sm3SourceModifier::DivideByZ {
                    2
                } else {
                    3
                };
                let plain = Sm3Source {
                    modifier: Sm3SourceModifier::None,
                    ..*source
                };
                format!("({base} / {})", self.source(&plain, divisor_lane)?)
            }
            Sm3SourceModifier::Absolute => format!("abs({base})"),
            Sm3SourceModifier::AbsoluteNegate => format!("(-abs({base}))"),
            Sm3SourceModifier::LogicalNot => format!("select(0.0, 1.0, {base} == 0.0)"),
        })
    }

    /// The register component `component` of the source, before any modifier.
    fn read(&self, source: &Sm3Source, component: u8) -> Result<String, Sm3WgslError> {
        let at_word = self.at_word;
        let register = source.register;
        let lane = component_name(component);
        if let Some(relative) = source.relative {
            let helper = self
                .relative_helper
                .ok_or(Sm3WgslError::RelativeConstantsNeedPassAbi)?;
            if register.file != Sm3RegisterFile::FloatConstant {
                return Err(Sm3WgslError::UnsupportedSourceFile {
                    at_word,
                    file: register.file,
                });
            }
            let written = self.defined.get(&relative.register).copied().unwrap_or(0);
            if written & (1 << relative.component) == 0 {
                return Err(Sm3WgslError::RegisterReadBeforeWrite {
                    at_word,
                    register: relative.register,
                    component: relative.component,
                });
            }
            return Ok(format!(
                "{helper}(sm3_constant_base, i32({}) + {}.{}).{lane}",
                register.index,
                register_name(relative.register),
                component_name(relative.component),
            ));
        }
        if let Some(&written) = self.defined.get(&register) {
            if written & (1 << component) == 0 {
                return Err(Sm3WgslError::RegisterReadBeforeWrite {
                    at_word,
                    register,
                    component,
                });
            }
        } else if !self.external.contains(&register) {
            return Err(match register.file {
                Sm3RegisterFile::Temporary | Sm3RegisterFile::Float16Temporary => {
                    Sm3WgslError::RegisterReadBeforeWrite {
                        at_word,
                        register,
                        component,
                    }
                }
                file => Sm3WgslError::UnsupportedSourceFile { at_word, file },
            });
        }
        Ok(format!("{}.{lane}", register_name(register)))
    }

    /// The value written to destination component `component`.
    pub fn operation(
        &self,
        opcode: Sm3Opcode,
        controls: u8,
        sources: &[Sm3Source],
        component: usize,
    ) -> Result<String, Sm3WgslError> {
        let at_word = self.at_word;
        let lane = |index: usize, lane: usize| self.source(&sources[index], lane);
        let each = |index: usize| lane(index, component);
        Ok(match opcode {
            Sm3Opcode::Mov => each(0)?,
            Sm3Opcode::Mova => format!("i32(round({}))", each(0)?),
            Sm3Opcode::Add => format!("({} + {})", each(0)?, each(1)?),
            Sm3Opcode::Sub => format!("({} - {})", each(0)?, each(1)?),
            Sm3Opcode::Mad => format!("({} * {} + {})", each(0)?, each(1)?, each(2)?),
            Sm3Opcode::Mul => format!("({} * {})", each(0)?, each(1)?),
            Sm3Opcode::Rcp => format!("(1.0 / {})", lane(0, 0)?),
            Sm3Opcode::Rsq => format!("inverseSqrt(abs({}))", lane(0, 0)?),
            Sm3Opcode::Dp3 => self.dot(sources, 3)?,
            Sm3Opcode::Dp4 => self.dot(sources, 4)?,
            Sm3Opcode::Min => format!("min({}, {})", each(0)?, each(1)?),
            Sm3Opcode::Max => format!("max({}, {})", each(0)?, each(1)?),
            Sm3Opcode::Slt => format!("select(0.0, 1.0, {} < {})", each(0)?, each(1)?),
            Sm3Opcode::Sge => format!("select(0.0, 1.0, {} >= {})", each(0)?, each(1)?),
            Sm3Opcode::Dsx => format!("dpdx({})", each(0)?),
            Sm3Opcode::Dsy => format!("dpdy({})", each(0)?),
            Sm3Opcode::Exp => format!("exp2({})", lane(0, 0)?),
            Sm3Opcode::Log => format!("log2(abs({}))", lane(0, 0)?),
            Sm3Opcode::Frc => format!("fract({})", each(0)?),
            Sm3Opcode::Abs => format!("abs({})", each(0)?),
            Sm3Opcode::Sincos => {
                let angle = lane(0, 0)?;
                match component {
                    0 => format!("cos({angle})"),
                    1 => format!("sin({angle})"),
                    _ => {
                        return Err(Sm3WgslError::InvalidSincosDestination {
                            at_word,
                            component: component as u8,
                        });
                    }
                }
            }
            Sm3Opcode::Cmp => {
                let value = each(0)?;
                let if_nonnegative = each(1)?;
                let if_negative = each(2)?;
                format!("select({if_negative}, {if_nonnegative}, {value} >= 0.0)")
            }
            Sm3Opcode::Lrp => {
                let amount = each(0)?;
                let toward = each(1)?;
                let from = each(2)?;
                format!("({amount} * {toward} + (1.0 - {amount}) * {from})")
            }
            Sm3Opcode::Nrm => {
                if component == 3 {
                    return Err(Sm3WgslError::InvalidNormaliseDestination {
                        at_word,
                        component: component as u8,
                    });
                }
                format!(
                    "normalize(vec3<f32>({}, {}, {}))[{component}]",
                    lane(0, 0)?,
                    lane(0, 1)?,
                    lane(0, 2)?
                )
            }
            Sm3Opcode::Pow => format!("pow(abs({}), {})", lane(0, 0)?, lane(1, 0)?),
            Sm3Opcode::Dp2Add => {
                let (x0, y0) = (lane(0, 0)?, lane(0, 1)?);
                let (x1, y1) = (lane(1, 0)?, lane(1, 1)?);
                format!("({x0} * {x1} + {y0} * {y1} + {})", lane(2, 0)?)
            }
            Sm3Opcode::TexLd | Sm3Opcode::TexLdL | Sm3Opcode::TexLdD => {
                self.texture(opcode, controls, sources, component)?
            }
            _ => {
                return Err(Sm3WgslError::UnsupportedInstructionControls {
                    at_word,
                    opcode,
                    controls,
                });
            }
        })
    }

    /// `a0*b0 + a1*b1 + ...` over the first `width` lanes of two sources.
    fn dot(&self, sources: &[Sm3Source], width: usize) -> Result<String, Sm3WgslError> {
        let left = (0..width)
            .map(|lane| self.source(&sources[0], lane))
            .collect::<Result<Vec<_>, _>>()?;
        let right = (0..width)
            .map(|lane| self.source(&sources[1], lane))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(left
            .iter()
            .zip(&right)
            .map(|(l, r)| format!("{l} * {r}"))
            .collect::<Vec<_>>()
            .join(" + "))
    }

    fn texture(
        &self,
        opcode: Sm3Opcode,
        controls: u8,
        sources: &[Sm3Source],
        component: usize,
    ) -> Result<String, Sm3WgslError> {
        let at_word = self.at_word;
        let invalid = Sm3WgslError::InvalidSamplerOperand { at_word };
        let sampler = sources.get(1).ok_or(invalid.clone())?;
        if sampler.register.file != Sm3RegisterFile::Sampler
            || sampler.modifier != Sm3SourceModifier::None
            || sampler.relative.is_some()
        {
            return Err(invalid);
        }
        let projected = opcode == Sm3Opcode::TexLd && controls == TEXLD_PROJECT;
        let x = self.source(&sources[0], 0)?;
        let y = self.source(&sources[0], 1)?;
        let dimension = *self.samplers.get(&sampler.register.index).ok_or(
            Sm3WgslError::SamplerDeclarationMissing {
                register: sampler.register.index,
            },
        )?;
        if projected && dimension == SamplerTextureDimension::Cube {
            return Err(Sm3WgslError::UnsupportedInstructionControls {
                at_word,
                opcode,
                controls,
            });
        }
        let w = if projected {
            Some(self.source(&sources[0], 3)?)
        } else {
            None
        };
        let coordinate = match (dimension, &w) {
            (SamplerTextureDimension::D2, Some(w)) => {
                format!("vec2<f32>({x} / {w}, {y} / {w})")
            }
            (SamplerTextureDimension::D2, None) => format!("vec2<f32>({x}, {y})"),
            (_, w) => {
                let z = self.source(&sources[0], 2)?;
                match w {
                    Some(w) => format!("vec3<f32>({x} / {w}, {y} / {w}, {z} / {w})"),
                    None => format!("vec3<f32>({x}, {y}, {z})"),
                }
            }
        };
        let slot = format!("sm3_slot_s{}", sampler.register.index);
        let texture = format!("{}[{slot} & 0xffffu]", texture_array_name(dimension));
        let sampler_expr = format!("sm3_samplers[{slot} >> 16u]");
        let call = match opcode {
            Sm3Opcode::TexLdL => {
                let level = self.source(&sources[0], 3)?;
                format!("textureSampleLevel({texture}, {sampler_expr}, {coordinate}, {level})")
            }
            Sm3Opcode::TexLdD => {
                if sources.len() < 4 {
                    return Err(Sm3WgslError::InvalidSamplerOperand { at_word });
                }
                let ddx = self.gradient(&sources[2], dimension)?;
                let ddy = self.gradient(&sources[3], dimension)?;
                format!(
                    "textureSampleGrad({texture}, {sampler_expr}, {coordinate}, {ddx}, {ddy})"
                )
            }
            _ => format!("textureSample({texture}, {sampler_expr}, {coordinate})"),
        };
        Ok(format!("{call}.{}", component_name(component as u8)))
    }

    fn gradient(
        &self,
        source: &Sm3Source,
        dimension: SamplerTextureDimension,
    ) -> Result<String, Sm3WgslError> {
        let x = self.source(source, 0)?;
        let y = self.source(source, 1)?;
        if dimension == SamplerTextureDimension::D2 {
            return Ok(format!("vec2<f32>({x}, {y})"));
        }
        let z = self.source(source, 2)?;
        Ok(format!("vec3<f32>({x}, {y}, {z})"))
    }
}
