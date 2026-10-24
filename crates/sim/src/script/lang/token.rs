//! Turning script text into tokens.

use super::fault::{Fault, Location};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// A name or keyword, lower-cased (scripts ignore case). A module path
    /// such as `maps\mp\_utility` is one word.
    Word(String),
    Int(i32),
    Float(f32),
    Text(String),
    Sym(&'static str),
    End,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub column: usize,
}

const PAIRS: [&str; 12] = [
    "::", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "++",
];
const LONE: [&str; 20] = [
    "--", "(", ")", "{", "}", "[", "]", ",", ";", ".", "=", "<", ">", "+", "-", "*", "/", "%", "!",
    "#",
];

struct Cursor<'a> {
    text: Vec<char>,
    at: usize,
    line: usize,
    column: usize,
    module: &'a str,
}

impl Cursor<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.text.get(self.at + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.at += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn fail(&self, message: &str) -> Fault {
        Fault::at(
            &Location {
                module: self.module.to_owned(),
                function: String::new(),
                line: self.line,
                column: self.column,
            },
            message,
        )
    }

    fn skip_blank(&mut self) -> Result<(), Fault> {
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some(c), _) if c.is_whitespace() => {
                    self.bump();
                }
                (Some('/'), Some('/')) => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                (Some('/'), Some('*')) => {
                    self.bump();
                    self.bump();
                    while !(self.peek(0) == Some('*') && self.peek(1) == Some('/')) {
                        if self.bump().is_none() {
                            return Err(self.fail("unterminated comment"));
                        }
                    }
                    self.bump();
                    self.bump();
                }
                _ => return Ok(()),
            }
        }
    }

    fn word(&mut self) -> String {
        let mut word = String::new();
        while let Some(c) = self.peek(0) {
            let path_step = c == '\\' && self.peek(1).is_some_and(|n| n.is_alphanumeric() || n == '_');
            if !(c.is_alphanumeric() || c == '_' || path_step) {
                break;
            }
            word.extend(c.to_lowercase());
            self.bump();
        }
        word
    }

    fn number(&mut self) -> Result<Tok, Fault> {
        let mut digits = String::new();
        let mut real = false;
        while let Some(c) = self.peek(0) {
            let fraction = c == '.' && !real && self.peek(1).is_some_and(|n| n.is_ascii_digit());
            if !(c.is_ascii_digit() || fraction) {
                break;
            }
            real |= fraction;
            digits.push(c);
            self.bump();
        }
        let bad = || self.fail(&format!("invalid number {digits}"));
        if real {
            digits.parse().map(Tok::Float).map_err(|_| bad())
        } else {
            digits.parse().map(Tok::Int).map_err(|_| bad())
        }
    }

    fn text(&mut self) -> Result<Tok, Fault> {
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.fail("unterminated string")),
                Some('"') => return Ok(Tok::Text(out)),
                Some('\\') => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => return Err(self.fail("unterminated string")),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn symbol(&mut self) -> Result<Tok, Fault> {
        let pair: String = [self.peek(0), self.peek(1)].into_iter().flatten().collect();
        let found = PAIRS
            .iter()
            .chain(LONE.iter())
            .find(|sym| pair.starts_with(*sym));
        match found {
            Some(&sym) => {
                (0..sym.len()).for_each(|_| {
                    self.bump();
                });
                Ok(Tok::Sym(sym))
            }
            None => Err(self.fail(&format!("unexpected character {:?}", self.peek(0)))),
        }
    }
}

pub fn lex(module: &str, source: &str) -> Result<Vec<Token>, Fault> {
    let mut cur = Cursor {
        text: source.chars().collect(),
        at: 0,
        line: 1,
        column: 1,
        module,
    };
    let mut out = Vec::new();
    loop {
        cur.skip_blank()?;
        let (line, column) = (cur.line, cur.column);
        let Some(c) = cur.peek(0) else { break };
        let tok = if c.is_alphabetic() || c == '_' {
            Tok::Word(cur.word())
        } else if c.is_ascii_digit() {
            cur.number()?
        } else if c == '"' {
            cur.text()?
        } else {
            cur.symbol()?
        };
        out.push(Token { tok, line, column });
    }
    out.push(Token {
        tok: Tok::End,
        line: cur.line,
        column: cur.column,
    });
    Ok(out)
}
