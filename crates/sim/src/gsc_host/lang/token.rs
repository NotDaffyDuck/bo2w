//! Turning script text into tokens.

use super::fault::{Fault, Location};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// A name or keyword, lower-cased (scripts ignore case). A module path
    /// such as `maps\mp\_utility` is one word.
    Word(String),
    Int(i32),
    Float(f32),
    Quoted(String),
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

struct TextWalk<'a> {
    text: Vec<char>,
    at: usize,
    line: usize,
    column: usize,
    module: &'a str,
}

impl TextWalk<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.text.get(self.at + ahead).copied()
    }

    fn step_char(&mut self) -> Option<char> {
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
                    self.step_char();
                }
                (Some('/'), Some('/')) => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.step_char();
                    }
                }
                (Some('/'), Some('*')) => {
                    self.step_char();
                    self.step_char();
                    while !(self.peek(0) == Some('*') && self.peek(1) == Some('/')) {
                        if self.step_char().is_none() {
                            return Err(self.fail("unterminated comment"));
                        }
                    }
                    self.step_char();
                    self.step_char();
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
            self.step_char();
        }
        word
    }

    fn number(&mut self) -> Result<Tok, Fault> {
        let mut numerals = String::new();
        let mut real = false;
        while let Some(c) = self.peek(0) {
            let fraction = c == '.' && !real && self.peek(1).is_some_and(|n| n.is_ascii_digit());
            if !(c.is_ascii_digit() || fraction) {
                break;
            }
            real |= fraction;
            numerals.push(c);
            self.step_char();
        }
        let bad = || self.fail(&format!("invalid number {numerals}"));
        if real {
            numerals.parse().map(Tok::Float).map_err(|_| bad())
        } else {
            numerals.parse().map(Tok::Int).map_err(|_| bad())
        }
    }

    fn text(&mut self) -> Result<Tok, Fault> {
        self.step_char();
        let mut out = String::new();
        loop {
            match self.step_char() {
                None => return Err(self.fail("unterminated string")),
                Some('"') => return Ok(Tok::Quoted(out)),
                Some('\\') => match self.step_char() {
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
        let got = PAIRS
            .iter()
            .chain(LONE.iter())
            .find(|sym| pair.starts_with(*sym));
        match got {
            Some(&sym) => {
                (0..sym.len()).for_each(|_| {
                    self.step_char();
                });
                Ok(Tok::Sym(sym))
            }
            None => Err(self.fail(&format!("unexpected character {:?}", self.peek(0)))),
        }
    }
}

pub fn lex(module: &str, source: &str) -> Result<Vec<Token>, Fault> {
    let mut cur = TextWalk {
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
