//! The instruction IR: every token of a Shader Model 3 program decoded into
//! typed operands, plus the check that a program only uses what the WGSL
//! emitter can lower.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::token::{ShaderStage, Token, TokenError, split_program};

/// Control bit on a texture load that divides the coordinate by its w.
pub(crate) const TEXLD_PROJECT: u8 = 1;

/// Raw opcodes the emitter can lower.
pub const SUPPORTED_OPCODE_SURFACE: &[u16] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x12, 0x13, 0x1f, 0x20, 0x23, 0x24, 0x25, 0x26, 0x27, 0x29, 0x2a, 0x2b, 0x2e, 0x30, 0x41, 0x42,
    0x51, 0x58, 0x5a, 0x5b, 0x5c, 0x5d, 0x5f,
];

/// Opcodes above 0x30 that the shader model also defines. Everything from
/// 0x00 to 0x30 is defined.
const NAMED_ABOVE_0X30: [u16; 16] = [
    0x40, 0x41, 0x42, 0x4e, 0x4f, 0x50, 0x51, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60,
];

/// How many opcodes the shader model names, for measuring how much of it is
/// supported.
pub fn named_sm3_opcode_count() -> usize {
    (0u16..=0x60)
        .filter(|raw| *raw <= 0x30 || NAMED_ABOVE_0X30.contains(raw))
        .count()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sm3RegisterFile {
    Temporary,
    Input,
    FloatConstant,
    Texture,
    Address,
    RasterOutput,
    AttributeOutput,
    Output,
    IntegerConstant,
    ColourOutput,
    DepthOutput,
    Sampler,
    BooleanConstant,
    Loop,
    Float16Temporary,
    Misc,
    Label,
    Predicate,
    Unknown(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sm3Register {
    pub file: Sm3RegisterFile,
    pub index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sm3SourceModifier {
    None,
    Negate,
    Bias,
    BiasNegate,
    Sign,
    SignNegate,
    Complement,
    Double,
    DoubleNegate,
    DivideByZ,
    DivideByW,
    Absolute,
    AbsoluteNegate,
    LogicalNot,
    Unknown(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sm3Opcode {
    Nop,
    Mov,
    Add,
    Sub,
    Mad,
    Mul,
    Rcp,
    Rsq,
    Dp3,
    Dp4,
    Min,
    Max,
    Slt,
    Sge,
    Exp,
    Log,
    Lrp,
    Frc,
    Abs,
    Nrm,
    Sincos,
    Rep,
    EndRep,
    Ifc,
    Else,
    Endif,
    Mova,
    DefI,
    TexKill,
    Pow,
    Dcl,
    Def,
    Cmp,
    Dp2Add,
    Dsx,
    Dsy,
    TexLd,
    TexLdD,
    TexLdL,
    Unknown(u16),
}

/// An address-register read that offsets a constant index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sm3RelativeAddress {
    pub register: Sm3Register,
    pub component: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sm3Destination {
    pub register: Sm3Register,
    /// One bit per written component, x in bit 0.
    pub write_mask: u8,
    pub saturate: bool,
    pub partial_precision: bool,
    pub centroid: bool,
    /// Result-modifier bits beyond the three named ones.
    pub unknown_modifier_bits: u8,
    pub relative: Option<Sm3RelativeAddress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sm3Source {
    pub register: Sm3Register,
    /// Which source component feeds each of x, y, z, w.
    pub swizzle: [u8; 4],
    pub modifier: Sm3SourceModifier,
    pub relative: Option<Sm3RelativeAddress>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sm3InstructionBody {
    Operation {
        destination: Option<Sm3Destination>,
        sources: Vec<Sm3Source>,
    },
    Declaration {
        usage_token: Option<u32>,
        destination: Sm3Destination,
    },
    FloatDefinition {
        destination: Sm3Destination,
        values: [u32; 4],
    },
    IntegerDefinition {
        destination: Sm3Destination,
        values: [u32; 4],
    },
    /// An opcode this IR does not know; the operand words are kept raw.
    Unmodeled {
        operands: Vec<u32>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sm3Instruction {
    pub at_word: usize,
    pub opcode: Sm3Opcode,
    pub controls: u8,
    pub predicated: bool,
    pub coissue: bool,
    pub body: Sm3InstructionBody,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sm3ProgramIr {
    pub stage: ShaderStage,
    pub instructions: Vec<Sm3Instruction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sm3IrError {
    TokenStream(TokenError),
    StageMismatch {
        expected: ShaderStage,
        actual: ShaderStage,
    },
    MissingOperand {
        at_word: usize,
        operand: usize,
    },
    MalformedRelativeAddress {
        at_word: usize,
        operand: usize,
    },
    InvalidDefinition {
        at_word: usize,
        operand_count: usize,
    },
    InvalidDeclaration {
        at_word: usize,
        operand_count: usize,
    },
    InvalidOperandCount {
        at_word: usize,
        opcode: Sm3Opcode,
        expected_sources: usize,
        actual_sources: usize,
    },
}

impl From<TokenError> for Sm3IrError {
    fn from(error: TokenError) -> Self {
        Self::TokenStream(error)
    }
}

/// Why a decoded program cannot be lowered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sm3LoweringRefusal {
    UnknownOpcode { at_word: usize, opcode: u16 },
    Predication { at_word: usize },
    Coissue { at_word: usize },
    RelativeAddressing { at_word: usize },
    UnknownRegisterFile { at_word: usize, file: u8 },
    UnknownSourceModifier { at_word: usize, modifier: u8 },
    UnknownDestinationModifier { at_word: usize, bits: u8 },
}

/// The operand layout an opcode takes.
#[derive(Clone, Copy)]
enum Shape {
    /// An optional destination, then exactly this many sources.
    Operation { sources: usize, destination: bool },
    Declaration,
    FloatDefinition,
    IntegerDefinition,
}

const fn op(sources: usize) -> Shape {
    Shape::Operation {
        sources,
        destination: true,
    }
}

const fn op_no_dest(sources: usize) -> Shape {
    Shape::Operation {
        sources,
        destination: false,
    }
}

const OPCODES: [(u16, Sm3Opcode, Shape); 39] = [
    (0x00, Sm3Opcode::Nop, op_no_dest(0)),
    (0x01, Sm3Opcode::Mov, op(1)),
    (0x02, Sm3Opcode::Add, op(2)),
    (0x03, Sm3Opcode::Sub, op(2)),
    (0x04, Sm3Opcode::Mad, op(3)),
    (0x05, Sm3Opcode::Mul, op(2)),
    (0x06, Sm3Opcode::Rcp, op(1)),
    (0x07, Sm3Opcode::Rsq, op(1)),
    (0x08, Sm3Opcode::Dp3, op(2)),
    (0x09, Sm3Opcode::Dp4, op(2)),
    (0x0a, Sm3Opcode::Min, op(2)),
    (0x0b, Sm3Opcode::Max, op(2)),
    (0x0c, Sm3Opcode::Slt, op(2)),
    (0x0d, Sm3Opcode::Sge, op(2)),
    (0x0e, Sm3Opcode::Exp, op(1)),
    (0x0f, Sm3Opcode::Log, op(1)),
    (0x12, Sm3Opcode::Lrp, op(3)),
    (0x13, Sm3Opcode::Frc, op(1)),
    (0x1f, Sm3Opcode::Dcl, Shape::Declaration),
    (0x20, Sm3Opcode::Pow, op(2)),
    (0x23, Sm3Opcode::Abs, op(1)),
    (0x24, Sm3Opcode::Nrm, op(1)),
    (0x25, Sm3Opcode::Sincos, op(1)),
    (0x26, Sm3Opcode::Rep, op_no_dest(1)),
    (0x27, Sm3Opcode::EndRep, op_no_dest(0)),
    (0x29, Sm3Opcode::Ifc, op_no_dest(2)),
    (0x2a, Sm3Opcode::Else, op_no_dest(0)),
    (0x2b, Sm3Opcode::Endif, op_no_dest(0)),
    (0x2e, Sm3Opcode::Mova, op(1)),
    (0x30, Sm3Opcode::DefI, Shape::IntegerDefinition),
    (0x41, Sm3Opcode::TexKill, op(0)),
    (0x42, Sm3Opcode::TexLd, op(2)),
    (0x51, Sm3Opcode::Def, Shape::FloatDefinition),
    (0x58, Sm3Opcode::Cmp, op(3)),
    (0x5a, Sm3Opcode::Dp2Add, op(3)),
    (0x5b, Sm3Opcode::Dsx, op(1)),
    (0x5c, Sm3Opcode::Dsy, op(1)),
    (0x5d, Sm3Opcode::TexLdD, op(4)),
    (0x5f, Sm3Opcode::TexLdL, op(2)),
];

fn lookup(raw: u16) -> Option<(Sm3Opcode, Shape)> {
    OPCODES
        .iter()
        .find(|entry| entry.0 == raw)
        .map(|entry| (entry.1, entry.2))
}

fn file_of(raw: u8) -> Sm3RegisterFile {
    use Sm3RegisterFile as F;
    match raw {
        0 => F::Temporary,
        1 => F::Input,
        2 | 11..=13 => F::FloatConstant,
        3 => F::Texture,
        4 => F::RasterOutput,
        5 => F::AttributeOutput,
        6 => F::Output,
        7 => F::IntegerConstant,
        8 => F::ColourOutput,
        9 => F::DepthOutput,
        10 => F::Sampler,
        14 => F::BooleanConstant,
        15 => F::Loop,
        16 => F::Float16Temporary,
        17 => F::Misc,
        18 => F::Label,
        19 => F::Predicate,
        other => F::Unknown(other),
    }
}

fn modifier_of(raw: u8) -> Sm3SourceModifier {
    use Sm3SourceModifier as M;
    match raw {
        0 => M::None,
        1 => M::Negate,
        2 => M::Bias,
        3 => M::BiasNegate,
        4 => M::Sign,
        5 => M::SignNegate,
        6 => M::Complement,
        7 => M::Double,
        8 => M::DoubleNegate,
        9 => M::DivideByZ,
        10 => M::DivideByW,
        11 => M::Absolute,
        12 => M::AbsoluteNegate,
        13 => M::LogicalNot,
        other => M::Unknown(other),
    }
}

/// Reads operand words of one instruction in order.
struct Operands<'a> {
    at_word: usize,
    words: &'a [u32],
    cursor: usize,
    stage: ShaderStage,
}

impl Operands<'_> {
    fn register(&self, token: u32) -> Sm3Register {
        let raw = (((token >> 28) & 0x7) | ((token >> 8) & 0x18)) as u8;
        let mut file = file_of(raw);
        // A vertex shader's t# register is the address register.
        if self.stage == ShaderStage::Vertex && file == Sm3RegisterFile::Texture {
            file = Sm3RegisterFile::Address;
        }
        Sm3Register {
            file,
            index: (token & 0x7ff) as u16,
        }
    }

    fn relative(&mut self, token: u32) -> Result<Option<Sm3RelativeAddress>, Sm3IrError> {
        if token & 0x2000 == 0 {
            return Ok(None);
        }
        let Some(&word) = self.words.get(self.cursor) else {
            return Err(Sm3IrError::MalformedRelativeAddress {
                at_word: self.at_word,
                operand: self.cursor,
            });
        };
        self.cursor += 1;
        Ok(Some(Sm3RelativeAddress {
            register: self.register(word),
            component: ((word >> 16) & 0x3) as u8,
        }))
    }

    fn take(&mut self) -> Result<u32, Sm3IrError> {
        let Some(&word) = self.words.get(self.cursor) else {
            return Err(Sm3IrError::MissingOperand {
                at_word: self.at_word,
                operand: self.cursor,
            });
        };
        self.cursor += 1;
        Ok(word)
    }

    fn destination(&mut self) -> Result<Sm3Destination, Sm3IrError> {
        let token = self.take()?;
        let modifiers = ((token >> 20) & 0xf) as u8;
        Ok(Sm3Destination {
            register: self.register(token),
            write_mask: ((token >> 16) & 0xf) as u8,
            saturate: modifiers & 1 != 0,
            partial_precision: modifiers & 2 != 0,
            centroid: modifiers & 4 != 0,
            unknown_modifier_bits: modifiers & !7,
            relative: self.relative(token)?,
        })
    }

    fn source(&mut self) -> Result<Sm3Source, Sm3IrError> {
        let token = self.take()?;
        let lanes = (token >> 16) as u8;
        Ok(Sm3Source {
            register: self.register(token),
            swizzle: [lanes & 3, (lanes >> 2) & 3, (lanes >> 4) & 3, lanes >> 6],
            modifier: modifier_of(((token >> 24) & 0xf) as u8),
            relative: self.relative(token)?,
        })
    }

    fn exhausted(&self) -> bool {
        self.cursor >= self.words.len()
    }
}

fn decode_instruction(token: &Token, stage: ShaderStage) -> Result<Sm3Instruction, Sm3IrError> {
    let at_word = token.at_word;
    let words = token.operands.as_slice();
    let mut reader = Operands {
        at_word,
        words,
        cursor: 0,
        stage,
    };
    let (opcode, body) = match lookup(token.opcode()) {
        None => (
            Sm3Opcode::Unknown(token.opcode()),
            Sm3InstructionBody::Unmodeled {
                operands: words.to_vec(),
            },
        ),
        Some((opcode, Shape::Operation {
            sources: expected,
            destination,
        })) => {
            let destination = if destination {
                Some(reader.destination()?)
            } else {
                None
            };
            let mut sources = Vec::new();
            while !reader.exhausted() {
                sources.push(reader.source()?);
            }
            if sources.len() != expected {
                return Err(Sm3IrError::InvalidOperandCount {
                    at_word,
                    opcode,
                    expected_sources: expected,
                    actual_sources: sources.len(),
                });
            }
            (
                opcode,
                Sm3InstructionBody::Operation {
                    destination,
                    sources,
                },
            )
        }
        Some((opcode, Shape::Declaration)) => {
            let usage_token = match words.len() {
                1 => None,
                2 => {
                    reader.cursor = 1;
                    Some(words[0])
                }
                operand_count => {
                    return Err(Sm3IrError::InvalidDeclaration {
                        at_word,
                        operand_count,
                    });
                }
            };
            let destination = reader.destination()?;
            if !reader.exhausted() {
                return Err(Sm3IrError::InvalidDeclaration {
                    at_word,
                    operand_count: words.len(),
                });
            }
            (
                opcode,
                Sm3InstructionBody::Declaration {
                    usage_token,
                    destination,
                },
            )
        }
        Some((opcode, shape @ (Shape::FloatDefinition | Shape::IntegerDefinition))) => {
            if words.len() != 5 {
                return Err(Sm3IrError::InvalidDefinition {
                    at_word,
                    operand_count: words.len(),
                });
            }
            let destination = reader.destination()?;
            let values = [words[1], words[2], words[3], words[4]];
            let body = if matches!(shape, Shape::FloatDefinition) {
                Sm3InstructionBody::FloatDefinition {
                    destination,
                    values,
                }
            } else {
                Sm3InstructionBody::IntegerDefinition {
                    destination,
                    values,
                }
            };
            (opcode, body)
        }
    };
    Ok(Sm3Instruction {
        at_word,
        opcode,
        controls: token.controls(),
        predicated: token.predicated(),
        coissue: token.coissue(),
        body,
    })
}

/// Decode a program, requiring it to be for `expected_stage`.
pub fn decode_sm3_program(
    bytes: &[u8],
    expected_stage: ShaderStage,
) -> Result<Sm3ProgramIr, Sm3IrError> {
    let (stage, tokens) = split_program(bytes)?;
    if stage != expected_stage {
        return Err(Sm3IrError::StageMismatch {
            expected: expected_stage,
            actual: stage,
        });
    }
    let instructions = tokens
        .iter()
        .map(|token| decode_instruction(token, stage))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Sm3ProgramIr {
        stage,
        instructions,
    })
}

/// A relative read is lowered only for a vertex shader reading a float
/// constant through the address register.
fn relative_unsupported(stage: ShaderStage, body: &Sm3InstructionBody) -> bool {
    match body {
        Sm3InstructionBody::Operation {
            destination,
            sources,
        } => {
            destination.is_some_and(|d| d.relative.is_some())
                || sources.iter().any(|source| match source.relative {
                    None => false,
                    Some(relative) => {
                        !(stage == ShaderStage::Vertex
                            && source.register.file == Sm3RegisterFile::FloatConstant
                            && relative.register.file == Sm3RegisterFile::Address)
                    }
                })
        }
        Sm3InstructionBody::Declaration { destination, .. }
        | Sm3InstructionBody::FloatDefinition { destination, .. }
        | Sm3InstructionBody::IntegerDefinition { destination, .. } => {
            destination.relative.is_some()
        }
        Sm3InstructionBody::Unmodeled { .. } => false,
    }
}

fn check_destination(at_word: usize, destination: &Sm3Destination) -> Result<(), Sm3LoweringRefusal> {
    if destination.unknown_modifier_bits != 0 {
        return Err(Sm3LoweringRefusal::UnknownDestinationModifier {
            at_word,
            bits: destination.unknown_modifier_bits,
        });
    }
    if let Sm3RegisterFile::Unknown(file) = destination.register.file {
        return Err(Sm3LoweringRefusal::UnknownRegisterFile { at_word, file });
    }
    Ok(())
}

fn check_source(at_word: usize, source: &Sm3Source) -> Result<(), Sm3LoweringRefusal> {
    if let Sm3RegisterFile::Unknown(file) = source.register.file {
        return Err(Sm3LoweringRefusal::UnknownRegisterFile { at_word, file });
    }
    if let Sm3SourceModifier::Unknown(modifier) = source.modifier {
        return Err(Sm3LoweringRefusal::UnknownSourceModifier { at_word, modifier });
    }
    Ok(())
}

/// Refuse the first thing in the program the emitter cannot lower.
pub fn validate_supported_opcode_surface(program: &Sm3ProgramIr) -> Result<(), Sm3LoweringRefusal> {
    for instruction in &program.instructions {
        let at_word = instruction.at_word;
        if instruction.predicated {
            return Err(Sm3LoweringRefusal::Predication { at_word });
        }
        if instruction.coissue {
            return Err(Sm3LoweringRefusal::Coissue { at_word });
        }
        if let Sm3Opcode::Unknown(opcode) = instruction.opcode {
            return Err(Sm3LoweringRefusal::UnknownOpcode { at_word, opcode });
        }
        if relative_unsupported(program.stage, &instruction.body) {
            return Err(Sm3LoweringRefusal::RelativeAddressing { at_word });
        }
        match &instruction.body {
            Sm3InstructionBody::Operation {
                destination,
                sources,
            } => {
                if let Some(destination) = destination {
                    check_destination(at_word, destination)?;
                }
                for source in sources {
                    check_source(at_word, source)?;
                }
            }
            Sm3InstructionBody::Declaration { destination, .. }
            | Sm3InstructionBody::FloatDefinition { destination, .. }
            | Sm3InstructionBody::IntegerDefinition { destination, .. } => {
                check_destination(at_word, destination)?;
            }
            Sm3InstructionBody::Unmodeled { .. } => {}
        }
    }
    Ok(())
}

/// How much of a body of shaders the emitter accepts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpcodeSurfaceCoverage {
    pub supported_opcodes: usize,
    pub named_sm3_opcodes: usize,
    pub programs_measured: usize,
    pub programs_passed: usize,
    pub programs_refused: usize,
    pub programs_decode_failed: usize,
    pub refused_unknown_opcodes: BTreeMap<u16, usize>,
    pub refused_other: BTreeMap<&'static str, usize>,
}

impl OpcodeSurfaceCoverage {
    /// The opcode counts alone, with no programs measured.
    pub fn surface_only() -> Self {
        Self {
            supported_opcodes: SUPPORTED_OPCODE_SURFACE.len(),
            named_sm3_opcodes: named_sm3_opcode_count(),
            ..Self::default()
        }
    }

    pub fn measure<'a>(programs: impl IntoIterator<Item = &'a Sm3ProgramIr>) -> Self {
        let mut coverage = Self::surface_only();
        for program in programs {
            coverage.tally(program);
        }
        coverage
    }

    pub fn measure_shaders<'a>(programs: impl IntoIterator<Item = (&'a [u8], ShaderStage)>) -> Self {
        let mut coverage = Self::surface_only();
        for (bytes, stage) in programs {
            match decode_sm3_program(bytes, stage) {
                Ok(program) => coverage.tally(&program),
                Err(_) => {
                    coverage.programs_measured = coverage.programs_measured.saturating_add(1);
                    coverage.programs_decode_failed =
                        coverage.programs_decode_failed.saturating_add(1);
                }
            }
        }
        coverage
    }

    fn tally(&mut self, program: &Sm3ProgramIr) {
        self.programs_measured = self.programs_measured.saturating_add(1);
        match validate_supported_opcode_surface(program) {
            Ok(()) => self.programs_passed = self.programs_passed.saturating_add(1),
            Err(refusal) => {
                self.programs_refused = self.programs_refused.saturating_add(1);
                self.record_refusal(&refusal);
            }
        }
    }

    fn record_refusal(&mut self, refusal: &Sm3LoweringRefusal) {
        let label = match *refusal {
            Sm3LoweringRefusal::UnknownOpcode { opcode, .. } => {
                let count = self.refused_unknown_opcodes.entry(opcode).or_insert(0);
                *count = count.saturating_add(1);
                return;
            }
            Sm3LoweringRefusal::Predication { .. } => "predication",
            Sm3LoweringRefusal::Coissue { .. } => "coissue",
            Sm3LoweringRefusal::RelativeAddressing { .. } => "relative-addressing",
            Sm3LoweringRefusal::UnknownRegisterFile { .. } => "unknown-register-file",
            Sm3LoweringRefusal::UnknownSourceModifier { .. } => "unknown-source-modifier",
            Sm3LoweringRefusal::UnknownDestinationModifier { .. } => "unknown-destination-modifier",
        };
        let count = self.refused_other.entry(label).or_insert(0);
        *count = count.saturating_add(1);
    }

    pub fn summary_line(&self) -> String {
        format!(
            "sm3 opcode surface: supported={}/{} named; corpus measured={} passed={} refused={} decode_failed={}; unknown_opcodes={:?}; other={:?}",
            self.supported_opcodes,
            self.named_sm3_opcodes,
            self.programs_measured,
            self.programs_passed,
            self.programs_refused,
            self.programs_decode_failed,
            self.refused_unknown_opcodes,
            self.refused_other,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS_3_0: u32 = 0xffff_0300;
    const VS_3_0: u32 = 0xfffe_0300;
    const END: u32 = 0x0000_ffff;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn the_surface_matches_the_opcode_table() {
        let table: Vec<u16> = OPCODES.iter().map(|entry| entry.0).collect();
        assert_eq!(table, SUPPORTED_OPCODE_SURFACE);
        assert_eq!(SUPPORTED_OPCODE_SURFACE.len(), 39);
        assert_eq!(named_sm3_opcode_count(), 65);
    }

    #[test]
    fn decodes_operands_into_typed_fields() {
        // mov r0.xz_sat, c3.wzyx with a negate modifier.
        let dest = 0x8000_0000 | (0b0101 << 16) | (1 << 20);
        let source = 0x8000_0000 | (1 << 24) | (0b00_01_10_11 << 16) | (2 << 28) | 3;
        let program = bytes(&[PS_3_0, 0x0200_0001, dest, source, END]);
        let ir = decode_sm3_program(&program, ShaderStage::Pixel).unwrap();
        let Sm3InstructionBody::Operation {
            destination: Some(destination),
            sources,
        } = &ir.instructions[0].body
        else {
            panic!("expected an operation");
        };
        assert_eq!(destination.register.file, Sm3RegisterFile::Temporary);
        assert_eq!(destination.write_mask, 0b0101);
        assert!(destination.saturate);
        assert_eq!(sources[0].register.file, Sm3RegisterFile::FloatConstant);
        assert_eq!(sources[0].register.index, 3);
        assert_eq!(sources[0].swizzle, [3, 2, 1, 0]);
        assert_eq!(sources[0].modifier, Sm3SourceModifier::Negate);
    }

    #[test]
    fn a_vertex_texture_register_is_the_address_register() {
        // mova a0.x, c0 then in a vertex shader t0 reads as a0.
        let dest = 0x8000_0000 | (3 << 28) | (1 << 16);
        let source = 0x8000_0000 | (2 << 28) | (0xe4 << 16);
        let program = bytes(&[VS_3_0, 0x0200_002e, dest, source, END]);
        let ir = decode_sm3_program(&program, ShaderStage::Vertex).unwrap();
        let Sm3InstructionBody::Operation {
            destination: Some(destination),
            ..
        } = &ir.instructions[0].body
        else {
            panic!("expected an operation");
        };
        assert_eq!(destination.register.file, Sm3RegisterFile::Address);
    }

    #[test]
    fn refuses_the_wrong_stage_and_a_wrong_operand_count() {
        let program = bytes(&[VS_3_0, END]);
        assert_eq!(
            decode_sm3_program(&program, ShaderStage::Pixel),
            Err(Sm3IrError::StageMismatch {
                expected: ShaderStage::Pixel,
                actual: ShaderStage::Vertex,
            })
        );
        let program = bytes(&[PS_3_0, 0x0100_0001, 0x800f_0800, END]);
        assert_eq!(
            decode_sm3_program(&program, ShaderStage::Pixel),
            Err(Sm3IrError::InvalidOperandCount {
                at_word: 1,
                opcode: Sm3Opcode::Mov,
                expected_sources: 1,
                actual_sources: 0,
            })
        );
    }

    #[test]
    fn unknown_opcodes_are_kept_raw_and_then_refused() {
        let program = bytes(&[PS_3_0, 0x0100_007e, 0x1234, END]);
        let ir = decode_sm3_program(&program, ShaderStage::Pixel).unwrap();
        assert_eq!(
            ir.instructions[0].body,
            Sm3InstructionBody::Unmodeled {
                operands: alloc::vec![0x1234]
            }
        );
        assert_eq!(
            validate_supported_opcode_surface(&ir),
            Err(Sm3LoweringRefusal::UnknownOpcode {
                at_word: 1,
                opcode: 0x7e
            })
        );
        let coverage = OpcodeSurfaceCoverage::measure([&ir]);
        assert_eq!(coverage.programs_refused, 1);
        assert_eq!(coverage.refused_unknown_opcodes[&0x7e], 1);
        assert!(coverage.summary_line().starts_with("sm3 opcode surface: supported=39/65"));
    }

    #[test]
    fn measure_shaders_counts_undecodable_programs() {
        let good = bytes(&[PS_3_0, END]);
        let bad = [1u8, 2, 3];
        let coverage = OpcodeSurfaceCoverage::measure_shaders([
            (good.as_slice(), ShaderStage::Pixel),
            (bad.as_slice(), ShaderStage::Pixel),
        ]);
        assert_eq!(coverage.programs_measured, 2);
        assert_eq!(coverage.programs_passed, 1);
        assert_eq!(coverage.programs_decode_failed, 1);
    }
}
