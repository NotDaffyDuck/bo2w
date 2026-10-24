//! First pass over a program: which registers it reads from outside, which it
//! writes, what it defines itself, and what it samples.

use alloc::collections::{BTreeMap, BTreeSet};

use super::Sm3WgslError;
use crate::abi::SamplerTextureDimension;
use crate::ir::{
    Sm3Destination, Sm3Instruction, Sm3InstructionBody, Sm3Opcode, Sm3ProgramIr, Sm3Register,
    Sm3RegisterFile, Sm3Source, TEXLD_PROJECT, validate_supported_opcode_surface,
};
use crate::token::ShaderStage;

#[derive(Default)]
pub(super) struct LoweringPlan {
    /// Registers the host feeds in: attributes, varyings, constants.
    pub external: BTreeSet<Sm3Register>,
    /// Which components of each external register are read.
    pub external_read_masks: BTreeMap<Sm3Register, u8>,
    /// Registers the program writes, each declared as a local variable.
    pub writable: BTreeSet<Sm3Register>,
    pub float_definitions: BTreeMap<Sm3Register, [u32; 4]>,
    pub integer_definitions: BTreeMap<Sm3Register, [u32; 4]>,
    pub samplers: BTreeMap<u16, SamplerTextureDimension>,
    pub output_masks: BTreeMap<Sm3Register, u8>,
    pub uses_relative_float: bool,
}

fn is_external(file: Sm3RegisterFile) -> bool {
    matches!(
        file,
        Sm3RegisterFile::Input
            | Sm3RegisterFile::FloatConstant
            | Sm3RegisterFile::Texture
            | Sm3RegisterFile::Misc
    )
}

fn is_output(file: Sm3RegisterFile) -> bool {
    matches!(
        file,
        Sm3RegisterFile::RasterOutput
            | Sm3RegisterFile::AttributeOutput
            | Sm3RegisterFile::Output
            | Sm3RegisterFile::ColourOutput
            | Sm3RegisterFile::DepthOutput
    )
}

fn is_writable(file: Sm3RegisterFile) -> bool {
    is_output(file)
        || matches!(
            file,
            Sm3RegisterFile::Temporary
                | Sm3RegisterFile::FloatConstant
                | Sm3RegisterFile::Float16Temporary
                | Sm3RegisterFile::Address
        )
}

pub(super) fn plan_program(program: &Sm3ProgramIr) -> Result<LoweringPlan, Sm3WgslError> {
    validate_supported_opcode_surface(program).map_err(Sm3WgslError::LoweringSurface)?;
    let mut plan = LoweringPlan::default();
    for instruction in &program.instructions {
        check_stage_and_controls(program.stage, instruction)?;
        let at_word = instruction.at_word;
        match &instruction.body {
            Sm3InstructionBody::Declaration {
                usage_token,
                destination,
            } => plan.declare(*usage_token, destination)?,
            Sm3InstructionBody::FloatDefinition {
                destination,
                values,
            } => {
                let register = constant_target(at_word, destination, Sm3RegisterFile::FloatConstant)?;
                plan.float_definitions.insert(register, *values);
            }
            Sm3InstructionBody::IntegerDefinition {
                destination,
                values,
            } => {
                let register =
                    constant_target(at_word, destination, Sm3RegisterFile::IntegerConstant)?;
                plan.integer_definitions.insert(register, *values);
            }
            Sm3InstructionBody::Operation {
                destination,
                sources,
            } => plan.operate(program.stage, instruction, destination.as_ref(), sources)?,
            Sm3InstructionBody::Unmodeled { .. } => {}
        }
    }
    for register in plan
        .float_definitions
        .keys()
        .chain(plan.integer_definitions.keys())
    {
        plan.external.remove(register);
        plan.writable.remove(register);
    }
    plan.uses_relative_float = program.instructions.iter().any(|instruction| {
        matches!(&instruction.body, Sm3InstructionBody::Operation { sources, .. }
            if sources.iter().any(|source| {
                source.relative.is_some()
                    && source.register.file == Sm3RegisterFile::FloatConstant
            }))
    });
    Ok(plan)
}

fn check_stage_and_controls(
    stage: ShaderStage,
    instruction: &Sm3Instruction,
) -> Result<(), Sm3WgslError> {
    let (at_word, opcode, controls) = (instruction.at_word, instruction.opcode, instruction.controls);
    let controls_ok = match opcode {
        Sm3Opcode::TexLd => controls <= TEXLD_PROJECT,
        Sm3Opcode::Ifc => controls <= 6,
        _ => controls == 0,
    };
    if !controls_ok {
        return Err(Sm3WgslError::UnsupportedInstructionControls {
            at_word,
            opcode,
            controls,
        });
    }
    let pixel_only = matches!(
        opcode,
        Sm3Opcode::TexKill | Sm3Opcode::Dsx | Sm3Opcode::Dsy | Sm3Opcode::TexLdD
    );
    if pixel_only && stage != ShaderStage::Pixel {
        return Err(Sm3WgslError::PixelOnlyOpcode { at_word, opcode });
    }
    if opcode == Sm3Opcode::Mova && stage != ShaderStage::Vertex {
        return Err(Sm3WgslError::VertexOnlyOpcode { at_word, opcode });
    }
    Ok(())
}

/// A definition must fill all four components of a register of `file`.
fn constant_target(
    at_word: usize,
    destination: &Sm3Destination,
    file: Sm3RegisterFile,
) -> Result<Sm3Register, Sm3WgslError> {
    let register = destination.register;
    if register.file != file {
        return Err(Sm3WgslError::UnsupportedDestinationFile {
            at_word,
            file: register.file,
        });
    }
    if destination.write_mask != 0x0f {
        return Err(Sm3WgslError::PartialConstantDefinition {
            at_word,
            register,
            write_mask: destination.write_mask,
        });
    }
    Ok(register)
}

impl LoweringPlan {
    fn declare(
        &mut self,
        usage_token: Option<u32>,
        destination: &Sm3Destination,
    ) -> Result<(), Sm3WgslError> {
        let register = destination.register;
        if register.file == Sm3RegisterFile::Sampler {
            let token = usage_token.ok_or(Sm3WgslError::SamplerDeclarationMissing {
                register: register.index,
            })?;
            let dimension = match (token >> 27) & 0xf {
                2 => SamplerTextureDimension::D2,
                3 => SamplerTextureDimension::Cube,
                4 => SamplerTextureDimension::D3,
                texture_type => {
                    return Err(Sm3WgslError::UnsupportedSamplerTextureType {
                        register: register.index,
                        texture_type,
                    });
                }
            };
            self.samplers.insert(register.index, dimension);
        } else if is_external(register.file) {
            self.external.insert(register);
        }
        Ok(())
    }

    fn operate(
        &mut self,
        stage: ShaderStage,
        instruction: &Sm3Instruction,
        destination: Option<&Sm3Destination>,
        sources: &[Sm3Source],
    ) -> Result<(), Sm3WgslError> {
        let at_word = instruction.at_word;
        let opcode = instruction.opcode;
        let destination_mask = destination.map_or(0x0f, |d| d.write_mask);
        for (index, source) in sources.iter().enumerate() {
            let register = source.register;
            let file = register.file;
            if opcode == Sm3Opcode::Rep && index == 0 {
                if file != Sm3RegisterFile::IntegerConstant
                    || source.modifier != crate::ir::Sm3SourceModifier::None
                    || source.relative.is_some()
                {
                    return Err(Sm3WgslError::UnsupportedSourceFile { at_word, file });
                }
                continue;
            }
            match file {
                Sm3RegisterFile::IntegerConstant => {
                    return Err(Sm3WgslError::UnsupportedSourceFile { at_word, file });
                }
                Sm3RegisterFile::Sampler => {
                    if !self.samplers.contains_key(&register.index) {
                        return Err(Sm3WgslError::SamplerDeclarationMissing {
                            register: register.index,
                        });
                    }
                }
                _ if is_external(file) => {
                    if file == Sm3RegisterFile::Misc
                        && !(stage == ShaderStage::Pixel && register.index <= 1)
                    {
                        return Err(Sm3WgslError::UnsupportedSourceFile { at_word, file });
                    }
                    // A relative read is bound by the whole constant block.
                    if source.relative.is_some() && file == Sm3RegisterFile::FloatConstant {
                        continue;
                    }
                    self.external.insert(register);
                    let dimension = sources
                        .get(1)
                        .filter(|sampler| sampler.register.file == Sm3RegisterFile::Sampler)
                        .and_then(|sampler| self.samplers.get(&sampler.register.index))
                        .copied();
                    let mask = source_read_mask(
                        opcode,
                        instruction.controls,
                        index,
                        destination_mask,
                        source,
                        dimension,
                    );
                    *self.external_read_masks.entry(register).or_insert(0) |= mask;
                }
                _ => {}
            }
        }

        let Some(destination) = destination else {
            return Ok(());
        };
        let register = destination.register;
        let file = register.file;
        if opcode == Sm3Opcode::TexKill {
            // The kill test reads its operand register rather than writing it.
            if is_external(file) {
                self.external.insert(register);
                *self.external_read_masks.entry(register).or_insert(0) |= 0x0f;
            } else if !matches!(
                file,
                Sm3RegisterFile::Temporary | Sm3RegisterFile::Float16Temporary
            ) {
                return Err(Sm3WgslError::UnsupportedDestinationFile { at_word, file });
            }
            return Ok(());
        }
        if (opcode == Sm3Opcode::Mova && file != Sm3RegisterFile::Address) || !is_writable(file) {
            return Err(Sm3WgslError::UnsupportedDestinationFile { at_word, file });
        }
        self.writable.insert(register);
        if is_output(file) {
            *self.output_masks.entry(register).or_insert(0) |= destination.write_mask;
        }
        Ok(())
    }
}

/// The source components an operation reads, as a mask over the register's
/// own components after the swizzle.
fn source_read_mask(
    opcode: Sm3Opcode,
    controls: u8,
    source_index: usize,
    destination_mask: u8,
    source: &Sm3Source,
    dimension: Option<SamplerTextureDimension>,
) -> u8 {
    use SamplerTextureDimension as D;
    let logical: u8 = match opcode {
        Sm3Opcode::Rcp
        | Sm3Opcode::Rsq
        | Sm3Opcode::Exp
        | Sm3Opcode::Log
        | Sm3Opcode::Pow
        | Sm3Opcode::Sincos
        | Sm3Opcode::Ifc
        | Sm3Opcode::Rep => 0x01,
        Sm3Opcode::Dp3 | Sm3Opcode::Nrm => 0x07,
        Sm3Opcode::Dp4 => 0x0f,
        Sm3Opcode::Dp2Add => {
            if source_index < 2 {
                0x03
            } else {
                0x01
            }
        }
        Sm3Opcode::TexLd => match (dimension, controls == TEXLD_PROJECT) {
            (Some(D::D2), true) => 0x0b,
            (Some(D::D2), false) => 0x03,
            (Some(D::D3), true) => 0x0f,
            (Some(D::Cube | D::D3), _) => 0x07,
            (None, _) => 0x0f,
        },
        Sm3Opcode::TexLdL => match dimension {
            Some(D::D2) => 0x0b,
            _ => 0x0f,
        },
        Sm3Opcode::TexLdD => match (source_index, dimension) {
            (1, _) => 0,
            (_, Some(D::D2)) => 0x03,
            (_, Some(D::Cube | D::D3)) => 0x07,
            (_, None) => 0x0f,
        },
        _ => destination_mask,
    };
    (0..4)
        .filter(|lane| logical & (1 << lane) != 0)
        .fold(0, |mask, lane| mask | 1 << source.swizzle[lane])
}
