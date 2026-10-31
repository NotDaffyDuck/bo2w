//! Running a function: a tree walk with a step budget.
//!
//! A script error never stops the world. It is handed to the host, the bad
//! expression reads as `undefined`, and the function carries on, the way the
//! game's own scripts behave. Only running out of budget ends a run.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::fault::{Fault, Location};
use super::program::Program;
use super::syntax::{Call, Expr, FnDef, Link, Op, Pos, Stmt};
use super::value::{Heap, Key, Value};

/// How many statements one run may execute before it is cut off.
const STEP_BUDGET: u64 = 16_000_000;
/// How deep function calls may nest.
const MAX_DEPTH: usize = 31;

/// What a running script reaches outside itself.
pub trait ScriptWorld {
    fn heap(&mut self) -> &mut Heap;

    /// Run engine command number `index` of the program.
    fn native(&mut self, slot_at: usize, receiver: &Value, args: &[Value]) -> Result<Value, String>;

    /// A field the engine owns (a player's health, say). `None` leaves the
    /// field to the heap.
    fn load_field(&mut self, object: u64, name: &str) -> Option<Value>;

    /// Hand a field write to the engine. `Ok(false)` means it is not an
    /// engine field and the heap should keep it.
    fn store_field(&mut self, object: u64, name: &str, value: &Value) -> Result<bool, String>;

    /// `receiver notify(event)` ran.
    fn raised(&mut self, receiver: &Value, event: &str);

    /// A script error happened; the run carries on.
    fn report(&mut self, fault: Fault);
}

enum Flow {
    Go,
    Break,
    Continue,
    Return(Value),
    /// The run was cut off.
    Kill,
}

struct Frame<'p> {
    def: &'p FnDef,
    me: Value,
    locals: BTreeMap<Arc<str>, Value>,
}

struct Machine<'a> {
    program: &'a Program,
    host: &'a mut dyn ScriptWorld,
    steps: u64,
    budget: u64,
    dead: bool,
    /// The call sites between the entry function and the running one.
    trail: Vec<Location>,
}

/// Run function `index` of `program` with `receiver` as `self`. `None` when
/// the run was cut off for taking too long.
pub fn play_call(
    program: &Program,
    host: &mut dyn ScriptWorld,
    index: usize,
    receiver: Value,
    args: Vec<Value>,
) -> Option<Value> {
    run_limited(program, host, index, receiver, args, STEP_BUDGET)
}

/// `play_call` with a chosen step budget.
pub(super) fn run_limited(
    program: &Program,
    host: &mut dyn ScriptWorld,
    index: usize,
    receiver: Value,
    args: Vec<Value>,
    budget: u64,
) -> Option<Value> {
    let mut machine = Machine {
        program,
        host,
        steps: 0,
        budget,
        dead: false,
        trail: Vec::new(),
    };
    let result = machine.invoke(index, receiver, args);
    (!machine.dead).then_some(result)
}

fn number(datum: &Value) -> Option<f32> {
    match datum {
        Value::Int(n) => Some(*n as f32),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

fn same(left: &Value, right: &Value) -> Result<bool, String> {
    use Value::*;
    Ok(match (left, right) {
        (Undefined, Undefined) => true,
        (Undefined, _) | (_, Undefined) => false,
        (Int(a), Int(b)) => a == b,
        (String(a), String(b)) => a == b,
        (Vector(a), Vector(b)) => a == b,
        (Object(a), Object(b)) | (Array(a), Array(b)) => a == b,
        (Object(_), Array(_)) | (Array(_), Object(_)) => false,
        _ => match (number(left), number(right)) {
            (Some(a), Some(b)) => a == b,
            _ => return Err(format!("cannot compare {} with {}", left.kind(), right.kind())),
        },
    })
}

fn arithmetic(op: Op, left: &Value, right: &Value) -> Result<Value, String> {
    use Value::*;
    let refuse = || {
        Err(format!(
            "cannot apply this operator to {} and {}",
            left.kind(),
            right.kind()
        ))
    };
    match (op, left, right) {
        (Op::Add, String(_), _) | (Op::Add, _, String(_)) => {
            match (left.to_text(), right.to_text()) {
                (Some(a), Some(b)) => Ok(Value::string(&(a + &b))),
                _ => refuse(),
            }
        }
        (_, Int(a), Int(b)) => match op {
            Op::Add => Ok(Int(a.wrapping_add(*b))),
            Op::Sub => Ok(Int(a.wrapping_sub(*b))),
            Op::Mul => Ok(Int(a.wrapping_mul(*b))),
            Op::Div | Op::Rem if *b == 0 => Err("division by zero".into()),
            Op::Div => Ok(Int(a.wrapping_div(*b))),
            _ => Ok(Int(a.wrapping_rem(*b))),
        },
        (Op::Add, Vector(a), Vector(b)) => Ok(Vector(std::array::from_fn(|i| a[i] + b[i]))),
        (Op::Sub, Vector(a), Vector(b)) => Ok(Vector(std::array::from_fn(|i| a[i] - b[i]))),
        (Op::Mul, Vector(v), scale) | (Op::Mul, scale, Vector(v)) => match number(scale) {
            Some(k) => Ok(Vector(v.map(|part| part * k))),
            None => refuse(),
        },
        (Op::Div, Vector(v), scale) => match number(scale) {
            Some(k) if k != 0.0 => Ok(Vector(v.map(|part| part / k))),
            Some(_) => Err("division by zero".into()),
            None => refuse(),
        },
        _ => match (number(left), number(right)) {
            (Some(a), Some(b)) => match op {
                Op::Add => Ok(Float(a + b)),
                Op::Sub => Ok(Float(a - b)),
                Op::Mul => Ok(Float(a * b)),
                Op::Div | Op::Rem if b == 0.0 => Err("division by zero".into()),
                Op::Div => Ok(Float(a / b)),
                _ => Ok(Float(a % b)),
            },
            _ => refuse(),
        },
    }
}

fn binary(op: Op, left: &Value, right: &Value) -> Result<Value, String> {
    let truth = |yes: bool| Ok(Value::Int(i32::from(yes)));
    match op {
        Op::Eq => truth(same(left, right)?),
        Op::Ne => truth(!same(left, right)?),
        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let (Some(a), Some(b)) = (number(left), number(right)) else {
                return Err(format!(
                    "cannot order {} and {}",
                    left.kind(),
                    right.kind()
                ));
            };
            truth(match op {
                Op::Lt => a < b,
                Op::Le => a <= b,
                Op::Gt => a > b,
                _ => a >= b,
            })
        }
        Op::And => truth(left.is_true() && right.is_true()),
        Op::Or => truth(left.is_true() || right.is_true()),
        _ => arithmetic(op, left, right),
    }
}

impl Machine<'_> {
    fn report(&mut self, frame: &Frame, pos: Pos, note: impl Into<String>) {
        let spot = Location {
            module: frame.def.module.clone(),
            function: frame.def.name.clone(),
            line: pos.line,
            column: pos.column,
        };
        let mut fault = Fault::at(&spot, note);
        fault.callers = self.trail.clone();
        self.host.report(fault);
    }

    fn invoke(&mut self, slot_at: usize, me: Value, args: Vec<Value>) -> Value {
        let def = self.program.function(slot_at);
        let mut frame = Frame {
            def,
            me,
            locals: def.params.iter().cloned().zip(args).collect(),
        };
        match self.block(&mut frame, &def.body) {
            Flow::Return(value) => value,
            _ => Value::Undefined,
        }
    }

    fn block(&mut self, frame: &mut Frame, body: &[Stmt]) -> Flow {
        for stmt in body {
            match self.stmt(frame, stmt) {
                Flow::Go => {}
                other => return other,
            }
        }
        Flow::Go
    }

    /// Count one unit of work; `false` once the run is out of budget.
    fn tick(&mut self, frame: &Frame, pos: Pos) -> bool {
        self.steps += 1;
        if self.steps > self.budget && !self.dead {
            self.dead = true;
            self.report(frame, pos, "potential infinite loop in script - killing thread");
        }
        !self.dead
    }

    fn stmt(&mut self, frame: &mut Frame, stmt: &Stmt) -> Flow {
        let pos = frame.def.pos;
        if !self.tick(frame, pos) {
            return Flow::Kill;
        }
        match stmt {
            Stmt::Eval(expr) => {
                self.expr(frame, expr);
                Flow::Go
            }
            Stmt::Assign { target, op, value, pos } => {
                let mut fresh = self.expr(frame, value);
                if let Some(op) = op {
                    let current = self.expr(frame, target);
                    match binary(*op, &current, &fresh) {
                        Ok(sum) => fresh = sum,
                        Err(message) => {
                            self.report(frame, *pos, message);
                            return Flow::Go;
                        }
                    }
                }
                self.assign(frame, target, fresh, *pos);
                Flow::Go
            }
            Stmt::If { cond, then, otherwise } => {
                if self.expr(frame, cond).is_true() {
                    self.stmt(frame, then)
                } else {
                    otherwise.as_ref().map_or(Flow::Go, |other| self.stmt(frame, other))
                }
            }
            Stmt::While { cond, body } => {
                while self.expr(frame, cond).is_true() {
                    if !self.tick(frame, pos) {
                        return Flow::Kill;
                    }
                    match self.stmt(frame, body) {
                        Flow::Go | Flow::Continue => {}
                        Flow::Break => break,
                        other => return other,
                    }
                }
                Flow::Go
            }
            Stmt::For { init, cond, step, body } => {
                if let Some(init) = init {
                    self.stmt(frame, init);
                }
                while cond.as_ref().is_none_or(|c| self.expr(frame, c).is_true()) {
                    if !self.tick(frame, pos) {
                        return Flow::Kill;
                    }
                    match self.stmt(frame, body) {
                        Flow::Go | Flow::Continue => {}
                        Flow::Break => break,
                        other => return other,
                    }
                    if let Some(step) = step {
                        self.stmt(frame, step);
                    }
                }
                Flow::Go
            }
            Stmt::Return(value) => {
                let value = value.as_ref().map_or(Value::Undefined, |e| self.expr(frame, e));
                Flow::Return(value)
            }
            Stmt::Break => Flow::Break,
            Stmt::Continue => Flow::Continue,
            Stmt::Block(body) => self.block(frame, body),
        }
    }

    fn assign(&mut self, frame: &mut Frame, aim_at: &Expr, value: Value, pos: Pos) {
        match aim_at {
            Expr::Local(name) => {
                frame.locals.insert(name.clone(), value);
            }
            Expr::Field(owner, name, _) => match self.expr(frame, owner) {
                Value::Object(object) => match self.host.store_field(object, name, &value) {
                    Ok(true) => {}
                    Ok(false) => self.host.heap().set_field(object, name, value),
                    Err(message) => self.report(frame, pos, message),
                },
                other => {
                    let message = format!("cannot set field {name} of {}", other.kind());
                    self.report(frame, pos, message);
                }
            },
            Expr::Index(owner, at, _) => {
                let (owner, at) = (self.expr(frame, owner), self.expr(frame, at));
                match (&owner, Key::of(&at)) {
                    (Value::Array(array), Some(key)) => {
                        self.host.heap().set_element(*array, key, value)
                    }
                    _ => {
                        let message =
                            format!("cannot set element {} of {}", at.kind(), owner.kind());
                        self.report(frame, pos, message);
                    }
                }
            }
            _ => self.report(frame, pos, "this cannot be assigned to"),
        }
    }

    fn expr(&mut self, frame: &mut Frame, expr: &Expr) -> Value {
        match expr {
            Expr::Lit(value) => value.clone(),
            Expr::Local(name) => frame.locals.get(name).cloned().unwrap_or(Value::Undefined),
            Expr::Me => frame.me.clone(),
            Expr::Root(object) => Value::Object(*object),
            Expr::NewArray => match self.host.heap().new_array() {
                Ok(array) => Value::Array(array),
                Err(message) => {
                    self.report(frame, frame.def.pos, message);
                    Value::Undefined
                }
            },
            Expr::Vector(parts) => {
                let mut axes = [0.0; 3];
                for (axis, part) in axes.iter_mut().zip(parts.iter()) {
                    match number(&self.expr(frame, part)) {
                        Some(x) => *axis = x,
                        None => {
                            self.report(frame, frame.def.pos, "vector parts must be numbers");
                            return Value::Undefined;
                        }
                    }
                }
                Value::Vector(axes)
            }
            Expr::Field(owner, name, pos) => {
                let owner = self.expr(frame, owner);
                self.extract_value(frame, &owner, name, *pos)
            }
            Expr::Index(owner, at, pos) => {
                let (owner, at) = (self.expr(frame, owner), self.expr(frame, at));
                let found = match (&owner, &at) {
                    (Value::Array(array), at) => Key::of(at)
                        .and_then(|key| self.host.heap().element(*array, &key).cloned()),
                    (Value::Vector(v), Value::Int(axis)) => {
                        usize::try_from(*axis).ok().and_then(|i| v.get(i)).map(|x| Value::Float(*x))
                    }
                    _ => {
                        let message =
                            format!("cannot index {} with {}", owner.kind(), at.kind());
                        self.report(frame, *pos, message);
                        None
                    }
                };
                found.unwrap_or(Value::Undefined)
            }
            Expr::Not(inner) => Value::Int(i32::from(!self.expr(frame, inner).is_true())),
            Expr::Neg(inner, pos) => match self.expr(frame, inner) {
                Value::Int(n) => Value::Int(n.wrapping_neg()),
                Value::Float(x) => Value::Float(-x),
                Value::Vector(v) => Value::Vector(v.map(|part| -part)),
                other => {
                    let message = format!("cannot negate {}", other.kind());
                    self.report(frame, *pos, message);
                    Value::Undefined
                }
            },
            Expr::Binary(op, left, right, pos) => {
                let left = self.expr(frame, left);
                let short = match op {
                    Op::And => !left.is_true(),
                    Op::Or => left.is_true(),
                    _ => false,
                };
                if short {
                    return Value::Int(i32::from(*op == Op::Or));
                }
                let right = self.expr(frame, right);
                binary(*op, &left, &right).unwrap_or_else(|message| {
                    self.report(frame, *pos, message);
                    Value::Undefined
                })
            }
            Expr::Call(call) => self.call(frame, call),
        }
    }

    fn extract_value(&mut self, frame: &Frame, owner: &Value, name: &str, pos: Pos) -> Value {
        let found = match owner {
            Value::Object(object) => self
                .host
                .load_field(*object, name)
                .or_else(|| self.host.heap().field(*object, name).cloned()),
            Value::Array(array) if name == "size" => {
                Some(Value::Int(self.host.heap().array_len(*array) as i32))
            }
            Value::String(text) if name == "size" => Some(Value::Int(text.chars().count() as i32)),
            other => {
                let message = format!("cannot read field {name} of {}", other.kind());
                self.report(frame, pos, message);
                None
            }
        };
        found.unwrap_or(Value::Undefined)
    }

    fn call(&mut self, frame: &mut Frame, call: &Call) -> Value {
        let receiver = call.receiver.as_ref().map(|e| self.expr(frame, e));
        let mut args = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            args.push(self.expr(frame, arg));
        }
        match call.link {
            Link::Script(index) => {
                if self.trail.len() >= MAX_DEPTH {
                    let message = "script stack overflow (too many embedded function calls)";
                    self.report(frame, call.pos, message);
                    return Value::Undefined;
                }
                self.trail.push(Location {
                    module: frame.def.module.clone(),
                    function: frame.def.name.clone(),
                    line: call.pos.line,
                    column: call.pos.column,
                });
                let me = receiver.unwrap_or_else(|| frame.me.clone());
                let result = self.invoke(index, me, args);
                self.trail.pop();
                result
            }
            Link::Native(index) => {
                let receiver = receiver.unwrap_or(Value::Undefined);
                self.host.native(index, &receiver, &args).unwrap_or_else(|message| {
                    self.report(frame, call.pos, message);
                    Value::Undefined
                })
            }
            Link::Notify => {
                let receiver = receiver.unwrap_or(Value::Undefined);
                match args.first() {
                    Some(Value::String(event)) => self.host.raised(&receiver, event),
                    _ => self.report(frame, call.pos, "notify needs an event name"),
                }
                Value::Undefined
            }
            Link::Pending => {
                self.report(frame, call.pos, "invalid IR: call was never linked");
                Value::Undefined
            }
        }
    }
}
