//! The shape of a parsed script.

use std::sync::Arc;

use super::value::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// What a call turned out to name once every module was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Pending,
    /// Index into the program's functions.
    Script(usize),
    /// Index into the program's engine commands.
    Native(usize),
    /// `object notify(event, ...)`: wakes nothing here, but a `level` notify
    /// is handed to the host.
    Notify,
}

#[derive(Clone, Debug)]
pub struct Call {
    /// The thing before the name in `thing name(args)`.
    pub receiver: Option<Expr>,
    pub module: Option<String>,
    pub name: String,
    pub args: Vec<Expr>,
    pub pos: Pos,
    pub link: Link,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(Value),
    Local(Arc<str>),
    /// `self`.
    Me,
    /// `level`, `game` or `anim`, as the heap handle they always have.
    Root(u64),
    Vector(Box<[Expr; 3]>),
    /// The empty array `[]`.
    NewArray,
    Field(Box<Expr>, Arc<str>, Pos),
    Index(Box<Expr>, Box<Expr>, Pos),
    Not(Box<Expr>),
    Neg(Box<Expr>, Pos),
    Binary(Op, Box<Expr>, Box<Expr>, Pos),
    Call(Box<Call>),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Eval(Expr),
    /// `target = value`, or `target += value` and the like when `op` is set.
    Assign {
        target: Expr,
        op: Option<Op>,
        value: Expr,
        pos: Pos,
    },
    If {
        cond: Expr,
        then: Box<Stmt>,
        otherwise: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        body: Box<Stmt>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Block(Vec<Stmt>),
}

#[derive(Clone, Debug)]
pub struct FnDef {
    pub module: String,
    pub name: String,
    pub params: Vec<Arc<str>>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
}
