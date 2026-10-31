//! Reading a script module into functions.

use std::sync::Arc;

use super::fault::{Fault, Location};
use super::syntax::{Call, Expr, FnDef, Link, Op, Pos, Stmt};
use super::token::{Tok, Token, lex};
use super::value::{ANIM, GAME, LEVEL, Value};

/// Words that start threads or wait on events. Nothing in this engine can
/// suspend a script, so they are refused rather than ignored.
const SUSPENDING: [&str; 9] = [
    "thread",
    "childthread",
    "wait",
    "waitframe",
    "waittill",
    "waittillmatch",
    "waittillframeend",
    "endon",
    "switch",
];

struct Reader {
    tokens: Vec<Token>,
    at: usize,
    module: String,
    function: String,
}

impl Reader {
    fn look(&self, ahead: usize) -> &Tok {
        let last = self.tokens.len() - 1;
        &self.tokens[(self.at + ahead).min(last)].tok
    }

    fn pos(&self) -> Pos {
        let token = &self.tokens[self.at.min(self.tokens.len() - 1)];
        Pos {
            line: token.line,
            column: token.column,
        }
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, Fault> {
        let pos = self.pos();
        Err(Fault::at(
            &Location {
                module: self.module.clone(),
                function: self.function.clone(),
                line: pos.line,
                column: pos.column,
            },
            message,
        ))
    }

    fn advance(&mut self) -> Tok {
        let tok = self.look(0).clone();
        self.at += 1;
        tok
    }

    fn is_sym(&self, sym: &str) -> bool {
        matches!(self.look(0), Tok::Sym(found) if *found == sym)
    }

    fn eat(&mut self, sym: &str) -> bool {
        let hit = self.is_sym(sym);
        if hit {
            self.at += 1;
        }
        hit
    }

    fn expect(&mut self, sym: &str) -> Result<(), Fault> {
        if self.eat(sym) {
            Ok(())
        } else {
            self.fail(format!("expected `{sym}`"))
        }
    }

    fn word(&mut self) -> Result<String, Fault> {
        match self.look(0) {
            Tok::Word(word) => {
                let word = word.clone();
                self.at += 1;
                Ok(word)
            }
            _ => self.fail("expected a name"),
        }
    }

    fn refuse_suspending(&self, word: &str) -> Result<(), Fault> {
        if SUSPENDING.contains(&word) {
            return self.fail(format!("`{word}` is not supported by this script engine"));
        }
        Ok(())
    }

    // ---- statements ----

    fn function(&mut self) -> Result<FnDef, Fault> {
        if self.is_sym("#") {
            return self.fail("`#include` and other directives are not supported");
        }
        let pos = self.pos();
        let name = self.word()?;
        self.function = name.clone();
        self.expect("(")?;
        let mut params: Vec<Arc<str>> = Vec::new();
        while !self.eat(")") {
            if !params.is_empty() {
                self.expect(",")?;
            }
            params.push(self.word()?.into());
        }
        let body = self.block()?;
        Ok(FnDef {
            module: self.module.clone(),
            name,
            params,
            body,
            pos,
        })
    }

    fn block(&mut self) -> Result<Vec<Stmt>, Fault> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.eat("}") {
            if *self.look(0) == Tok::End {
                return self.fail("expected `}`");
            }
            body.push(self.statement()?);
        }
        Ok(body)
    }

    fn statement(&mut self) -> Result<Stmt, Fault> {
        if self.is_sym("{") {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.eat(";") {
            return Ok(Stmt::Block(Vec::new()));
        }
        if let Tok::Word(word) = self.look(0).clone() {
            self.refuse_suspending(&word)?;
            match word.as_str() {
                "if" => return self.conditional(),
                "while" => return self.while_loop(),
                "for" => return self.for_loop(),
                "return" => {
                    self.at += 1;
                    let value = if self.is_sym(";") { None } else { Some(self.expression()?) };
                    self.expect(";")?;
                    return Ok(Stmt::Return(value));
                }
                "break" | "continue" => {
                    self.at += 1;
                    self.expect(";")?;
                    return Ok(if word == "break" { Stmt::Break } else { Stmt::Continue });
                }
                _ => {}
            }
        }
        let simple = self.simple()?;
        self.expect(";")?;
        Ok(simple)
    }

    fn conditional(&mut self) -> Result<Stmt, Fault> {
        self.at += 1;
        self.expect("(")?;
        let cond = self.expression()?;
        self.expect(")")?;
        let then = Box::new(self.statement()?);
        let otherwise = if matches!(self.look(0), Tok::Word(w) if w == "else") {
            self.at += 1;
            Some(Box::new(self.statement()?))
        } else {
            None
        };
        Ok(Stmt::If { cond, then, otherwise })
    }

    fn while_loop(&mut self) -> Result<Stmt, Fault> {
        self.at += 1;
        self.expect("(")?;
        let cond = self.expression()?;
        self.expect(")")?;
        Ok(Stmt::While {
            cond,
            body: Box::new(self.statement()?),
        })
    }

    fn for_loop(&mut self) -> Result<Stmt, Fault> {
        self.at += 1;
        self.expect("(")?;
        let init = if self.is_sym(";") { None } else { Some(Box::new(self.simple()?)) };
        self.expect(";")?;
        let cond = if self.is_sym(";") { None } else { Some(self.expression()?) };
        self.expect(";")?;
        let step = if self.is_sym(")") { None } else { Some(Box::new(self.simple()?)) };
        self.expect(")")?;
        Ok(Stmt::For {
            init,
            cond,
            step,
            body: Box::new(self.statement()?),
        })
    }

    /// An expression on its own, or an assignment (no trailing `;`).
    fn simple(&mut self) -> Result<Stmt, Fault> {
        let pos = self.pos();
        let target = self.expression()?;
        let op = match self.look(0) {
            Tok::Sym("=") => None,
            Tok::Sym("+=") => Some(Op::Add),
            Tok::Sym("-=") => Some(Op::Sub),
            Tok::Sym("*=") => Some(Op::Mul),
            Tok::Sym("/=") => Some(Op::Div),
            Tok::Sym("++") | Tok::Sym("--") => {
                let op = if self.is_sym("++") { Op::Add } else { Op::Sub };
                self.at += 1;
                return self.assignment(target, Some(op), Expr::Lit(Value::Int(1)), pos);
            }
            _ => return Ok(Stmt::Eval(target)),
        };
        self.at += 1;
        let value = self.expression()?;
        self.assignment(target, op, value, pos)
    }

    fn assignment(&self, aim_at: Expr, op: Option<Op>, value: Expr, pos: Pos) -> Result<Stmt, Fault> {
        match aim_at {
            Expr::Local(_) | Expr::Field(..) | Expr::Index(..) => {
                Ok(Stmt::Assign { target: aim_at, op, value, pos })
            }
            _ => self.fail("this cannot be assigned to"),
        }
    }

    // ---- expressions, loosest binding first ----

    fn expression(&mut self) -> Result<Expr, Fault> {
        self.binary_level(0)
    }

    fn binary_level(&mut self, level: usize) -> Result<Expr, Fault> {
        const LEVELS: [&[(&str, Op)]; 5] = [
            &[("||", Op::Or)],
            &[("&&", Op::And)],
            &[("==", Op::Eq), ("!=", Op::Ne)],
            &[("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)],
            &[("+", Op::Add), ("-", Op::Sub)],
        ];
        let Some(ops) = LEVELS.get(level) else {
            return self.product();
        };
        let mut left = self.binary_level(level + 1)?;
        loop {
            let pos = self.pos();
            let Some(&(_, op)) = ops.iter().find(|(sym, _)| self.is_sym(sym)) else {
                return Ok(left);
            };
            self.at += 1;
            let right = self.binary_level(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right), pos);
        }
    }

    fn product(&mut self) -> Result<Expr, Fault> {
        let mut left = self.unary()?;
        loop {
            let pos = self.pos();
            let op = match self.look(0) {
                Tok::Sym("*") => Op::Mul,
                Tok::Sym("/") => Op::Div,
                Tok::Sym("%") => Op::Rem,
                _ => return Ok(left),
            };
            self.at += 1;
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right), pos);
        }
    }

    fn unary(&mut self) -> Result<Expr, Fault> {
        let pos = self.pos();
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?), pos));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, Fault> {
        let mut left = self.primary()?;
        loop {
            let pos = self.pos();
            if self.eat(".") {
                let name = self.word()?;
                left = Expr::Field(Box::new(left), name.into(), pos);
            } else if self.eat("[") {
                let index = self.expression()?;
                self.expect("]")?;
                left = Expr::Index(Box::new(left), Box::new(index), pos);
            } else if matches!(self.look(0), Tok::Word(_)) {
                left = self.method(left)?;
            } else {
                return Ok(left);
            }
        }
    }

    /// `receiver name(args)` or `receiver module::name(args)`.
    fn method(&mut self, receiver: Expr) -> Result<Expr, Fault> {
        let pos = self.pos();
        let lead = self.word()?;
        self.refuse_suspending(&lead)?;
        let (module, name) = if self.eat("::") {
            (Some(lead), self.word()?)
        } else {
            (None, lead)
        };
        self.call(Some(receiver), module, name, pos)
    }

    fn call(&mut self, receiver: Option<Expr>, module: Option<String>, called: String, pos: Pos) -> Result<Expr, Fault> {
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.eat(")") {
            if !args.is_empty() {
                self.expect(",")?;
            }
            args.push(self.expression()?);
        }
        Ok(Expr::Call(Box::new(Call {
            receiver,
            module,
            name: called,
            args,
            pos,
            link: Link::Pending,
        })))
    }

    fn primary(&mut self) -> Result<Expr, Fault> {
        let pos = self.pos();
        match self.advance() {
            Tok::Int(n) => Ok(Expr::Lit(Value::Int(n))),
            Tok::Float(x) => Ok(Expr::Lit(Value::Float(x))),
            Tok::Quoted(text) => Ok(Expr::Lit(Value::string(&text))),
            Tok::Sym("(") => {
                let first = self.expression()?;
                if !self.eat(",") {
                    self.expect(")")?;
                    return Ok(first);
                }
                let second = self.expression()?;
                self.expect(",")?;
                let third = self.expression()?;
                self.expect(")")?;
                Ok(Expr::Vector(Box::new([first, second, third])))
            }
            Tok::Sym("[") => {
                self.expect("]")?;
                Ok(Expr::NewArray)
            }
            Tok::Word(word) => {
                self.refuse_suspending(&word)?;
                match word.as_str() {
                    "true" => return Ok(Expr::Lit(Value::Int(1))),
                    "false" => return Ok(Expr::Lit(Value::Int(0))),
                    "undefined" => return Ok(Expr::Lit(Value::Undefined)),
                    "self" => return Ok(Expr::Me),
                    "level" => return Ok(Expr::Root(LEVEL)),
                    "game" => return Ok(Expr::Root(GAME)),
                    "anim" => return Ok(Expr::Root(ANIM)),
                    _ => {}
                }
                if self.eat("::") {
                    let name = self.word()?;
                    return self.call(None, Some(word), name, pos);
                }
                if self.is_sym("(") {
                    return self.call(None, None, word, pos);
                }
                Ok(Expr::Local(word.into()))
            }
            _ => {
                self.at -= 1;
                self.fail("expected an expression")
            }
        }
    }
}

/// Every function in one module's text, in file order.
pub fn parse(module: &str, source: &str) -> Result<Vec<FnDef>, Fault> {
    let mut reader = Reader {
        tokens: lex(module, source)?,
        at: 0,
        module: module.to_owned(),
        function: String::new(),
    };
    let mut functions = Vec::new();
    while *reader.look(0) != Tok::End {
        functions.push(reader.function()?);
    }
    Ok(functions)
}
