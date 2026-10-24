//! The instruction stream of one stage: assignments, branches and loops.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::expr::Scope;
use super::plan::LoweringPlan;
use super::{Sm3WgslError, component_name, register_name};
use crate::ir::{
    Sm3Destination, Sm3Instruction, Sm3InstructionBody, Sm3Opcode, Sm3ProgramIr, Sm3Register,
    Sm3Source, Sm3SourceModifier,
};

/// Components of each register written on every path so far.
pub(super) type Defined = BTreeMap<Sm3Register, u8>;

enum Frame {
    If {
        at_word: usize,
        /// What was written when the branch opened.
        at_open: Defined,
        /// What the `then` side had written, once an `else` has started.
        then_end: Option<Defined>,
    },
    Rep {
        at_word: usize,
        at_open: Defined,
    },
}

impl Frame {
    fn at_word(&self) -> usize {
        match self {
            Self::If { at_word, .. } | Self::Rep { at_word, .. } => *at_word,
        }
    }

    fn unclosed_kind(&self) -> &'static str {
        match self {
            Self::If { .. } => "unclosed-ifc",
            Self::Rep { .. } => "unclosed-rep",
        }
    }
}

/// What a branch leaves written: everything from before, plus what both sides
/// wrote.
fn merge(before: &Defined, then_end: &Defined, else_end: &Defined) -> Defined {
    let mut merged = before.clone();
    for register in then_end.keys().chain(else_end.keys()) {
        let kept = before.get(register).copied().unwrap_or(0)
            | (then_end.get(register).copied().unwrap_or(0)
                & else_end.get(register).copied().unwrap_or(0));
        if kept == 0 {
            merged.remove(register);
        } else {
            merged.insert(*register, kept);
        }
    }
    merged
}

struct Body<'a> {
    plan: &'a LoweringPlan,
    relative_helper: Option<&'a str>,
    defined: Defined,
    stack: Vec<Frame>,
}

/// Write the statements of `program` into `out`, starting from the registers
/// already `defined` by the prologue.
pub(super) fn emit_body(
    out: &mut String,
    program: &Sm3ProgramIr,
    plan: &LoweringPlan,
    defined: Defined,
    relative_helper: Option<&str>,
) -> Result<(), Sm3WgslError> {
    let mut body = Body {
        plan,
        relative_helper,
        defined,
        stack: Vec::new(),
    };
    for instruction in &program.instructions {
        body.instruction(out, instruction)?;
    }
    match body.stack.last() {
        Some(frame) => Err(Sm3WgslError::UnbalancedControlFlow {
            at_word: frame.at_word(),
            kind: frame.unclosed_kind(),
        }),
        None => Ok(()),
    }
}

impl Body<'_> {
    fn scope(&self, at_word: usize) -> Scope<'_> {
        Scope {
            at_word,
            external: &self.plan.external,
            defined: &self.defined,
            samplers: &self.plan.samplers,
            relative_helper: self.relative_helper,
        }
    }

    fn instruction(
        &mut self,
        out: &mut String,
        instruction: &Sm3Instruction,
    ) -> Result<(), Sm3WgslError> {
        let Sm3InstructionBody::Operation {
            destination,
            sources,
        } = &instruction.body
        else {
            return Ok(());
        };
        let at_word = instruction.at_word;
        match instruction.opcode {
            Sm3Opcode::Rep => self.open_repeat(out, at_word, sources),
            Sm3Opcode::EndRep => self.close_repeat(out, at_word),
            Sm3Opcode::Ifc => self.open_branch(out, at_word, instruction.controls, sources),
            Sm3Opcode::Else => self.start_else(out, at_word),
            Sm3Opcode::Endif => self.close_branch(out, at_word),
            _ => match destination {
                Some(destination) => self.assign(out, instruction, destination, sources),
                None => Ok(()),
            },
        }
    }

    fn open_repeat(
        &mut self,
        out: &mut String,
        at_word: usize,
        sources: &[Sm3Source],
    ) -> Result<(), Sm3WgslError> {
        let counter = &sources[0];
        let component = counter.swizzle[0];
        let written = self.defined.get(&counter.register).copied().unwrap_or(0);
        if written & (1 << component) == 0 {
            return Err(Sm3WgslError::RegisterReadBeforeWrite {
                at_word,
                register: counter.register,
                component,
            });
        }
        emit!(
            out,
            "    var sm3_rep_{at_word} = bitcast<u32>({}.{});\n    loop {{\n    if sm3_rep_{at_word} == 0u {{ break; }}\n    sm3_rep_{at_word} -= 1u;\n",
            register_name(counter.register),
            component_name(component),
        );
        self.stack.push(Frame::Rep {
            at_word,
            at_open: self.defined.clone(),
        });
        Ok(())
    }

    fn close_repeat(&mut self, out: &mut String, at_word: usize) -> Result<(), Sm3WgslError> {
        match self.stack.pop() {
            Some(Frame::Rep { at_open, .. }) => {
                self.defined = at_open;
                out.push_str("    }\n");
                Ok(())
            }
            Some(Frame::If { .. }) => unbalanced(at_word, "endrep-crosses-ifc"),
            None => unbalanced(at_word, "endrep-without-rep"),
        }
    }

    fn open_branch(
        &mut self,
        out: &mut String,
        at_word: usize,
        controls: u8,
        sources: &[Sm3Source],
    ) -> Result<(), Sm3WgslError> {
        let comparison = match controls {
            1 => ">",
            2 => "==",
            3 => ">=",
            4 => "<",
            5 => "!=",
            6 => "<=",
            _ => {
                return Err(Sm3WgslError::UnsupportedInstructionControls {
                    at_word,
                    opcode: Sm3Opcode::Ifc,
                    controls,
                });
            }
        };
        let scope = self.scope(at_word);
        let left = scope.source(&sources[0], 0)?;
        let right = scope.source(&sources[1], 0)?;
        emit!(out, "    if ({left} {comparison} {right}) {{\n");
        self.stack.push(Frame::If {
            at_word,
            at_open: self.defined.clone(),
            then_end: None,
        });
        Ok(())
    }

    fn start_else(&mut self, out: &mut String, at_word: usize) -> Result<(), Sm3WgslError> {
        match self.stack.last_mut() {
            Some(Frame::If {
                at_open, then_end, ..
            }) => {
                if then_end.is_some() {
                    return unbalanced(at_word, "double-else");
                }
                *then_end = Some(core::mem::replace(&mut self.defined, at_open.clone()));
                out.push_str("    } else {\n");
                Ok(())
            }
            Some(Frame::Rep { .. }) => unbalanced(at_word, "else-crosses-rep"),
            None => unbalanced(at_word, "else-without-ifc"),
        }
    }

    fn close_branch(&mut self, out: &mut String, at_word: usize) -> Result<(), Sm3WgslError> {
        match self.stack.pop() {
            Some(Frame::If {
                at_open, then_end, ..
            }) => {
                // Without an `else` the missing side wrote nothing new.
                let (then_end, else_end) = match then_end {
                    Some(then_end) => (then_end, self.defined.clone()),
                    None => (self.defined.clone(), at_open.clone()),
                };
                self.defined = merge(&at_open, &then_end, &else_end);
                out.push_str("    }\n");
                Ok(())
            }
            Some(Frame::Rep { .. }) => unbalanced(at_word, "endif-crosses-rep"),
            None => unbalanced(at_word, "endif-without-ifc"),
        }
    }

    fn assign(
        &mut self,
        out: &mut String,
        instruction: &Sm3Instruction,
        destination: &Sm3Destination,
        sources: &[Sm3Source],
    ) -> Result<(), Sm3WgslError> {
        let at_word = instruction.at_word;
        if instruction.opcode == Sm3Opcode::TexKill {
            return self.kill(out, at_word, destination);
        }
        let target = register_name(destination.register);
        let aliased = sources
            .iter()
            .any(|source| source.register == destination.register);
        let scope = self.scope(at_word);
        let mut values = Vec::new();
        for lane in 0..4usize {
            if destination.write_mask & (1 << lane) == 0 {
                continue;
            }
            let mut value =
                scope.operation(instruction.opcode, instruction.controls, sources, lane)?;
            if destination.saturate {
                if instruction.opcode == Sm3Opcode::Mova {
                    return Err(Sm3WgslError::SaturateOnAddress { at_word });
                }
                value = format!("clamp({value}, 0.0, 1.0)");
            }
            values.push((lane as u8, value));
        }
        // A source that is also the destination is read in full before any
        // component of it is overwritten.
        for (lane, value) in &values {
            let name = component_name(*lane);
            if aliased {
                emit!(out, "    let sm3_value_{at_word}_{name} = {value};\n");
            } else {
                emit!(out, "    {target}.{name} = {value};\n");
            }
        }
        if aliased {
            for (lane, _) in &values {
                let name = component_name(*lane);
                emit!(out, "    {target}.{name} = sm3_value_{at_word}_{name};\n");
            }
        }
        *self.defined.entry(destination.register).or_insert(0) |= destination.write_mask;
        Ok(())
    }

    /// `texkill`: discard when any of the first four components is negative.
    fn kill(
        &self,
        out: &mut String,
        at_word: usize,
        destination: &Sm3Destination,
    ) -> Result<(), Sm3WgslError> {
        let operand = Sm3Source {
            register: destination.register,
            swizzle: [0, 1, 2, 3],
            modifier: Sm3SourceModifier::None,
            relative: None,
        };
        let scope = self.scope(at_word);
        let x = scope.source(&operand, 0)?;
        let y = scope.source(&operand, 1)?;
        let z = scope.source(&operand, 2)?;
        let w = scope.source(&operand, 3)?;
        emit!(
            out,
            "    if ({x} < 0.0 || {y} < 0.0 || {z} < 0.0 || {w} < 0.0) {{ discard; }}\n"
        );
        Ok(())
    }
}

fn unbalanced(at_word: usize, kind: &'static str) -> Result<(), Sm3WgslError> {
    Err(Sm3WgslError::UnbalancedControlFlow { at_word, kind })
}
