//! Splitting a Shader Model 3 binary into instruction tokens.
//!
//! The binary is little-endian 32-bit words: a version word, then one token per
//! instruction followed by its operand words, then an END token. Comment
//! blocks are skipped here, so what comes out is only real instructions.

use alloc::vec::Vec;
use core::fmt;

const END_TOKEN: u16 = 0xffff;
const COMMENT_TOKEN: u16 = 0xfffe;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShaderStage {
    Pixel,
    Vertex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    Empty,
    UnalignedBytes(usize),
    UnsupportedVersion(u32),
    Truncated {
        at_word: usize,
        needed: usize,
        have: usize,
    },
    MissingEnd,
    TrailingWords(usize),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => f.write_str("shader has no version token"),
            Self::UnalignedBytes(len) => write!(f, "shader length {len} is not DWORD-aligned"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported shader version {version:#010x}")
            }
            Self::Truncated {
                at_word,
                needed,
                have,
            } => write!(
                f,
                "token at word {at_word} needs {needed} words, only {have} remain"
            ),
            Self::MissingEnd => f.write_str("shader has no END token"),
            Self::TrailingWords(words) => write!(f, "END is followed by {words} words"),
        }
    }
}

/// One instruction: its token word and the operand words after it.
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub at_word: usize,
    pub word: u32,
    pub operands: Vec<u32>,
}

impl Token {
    pub fn opcode(&self) -> u16 {
        self.word as u16
    }

    /// The opcode-specific control bits (comparison kind, projection flag).
    pub fn controls(&self) -> u8 {
        (self.word >> 16) as u8
    }

    pub fn predicated(&self) -> bool {
        self.word & 0x1000_0000 != 0
    }

    pub fn coissue(&self) -> bool {
        self.word & 0x4000_0000 != 0
    }
}

fn stage_of(version: u32) -> Result<ShaderStage, TokenError> {
    let stage = match version >> 16 {
        0xffff => ShaderStage::Pixel,
        0xfffe => ShaderStage::Vertex,
        _ => return Err(TokenError::UnsupportedVersion(version)),
    };
    let (major, minor) = ((version >> 8) & 0xff, version & 0xff);
    if (major, minor) == (3, 0) {
        Ok(stage)
    } else {
        Err(TokenError::UnsupportedVersion(version))
    }
}

/// How many operand words follow a token.
fn operand_count(word: u32) -> usize {
    if word as u16 == COMMENT_TOKEN {
        ((word >> 16) & 0x7fff) as usize
    } else {
        ((word >> 24) & 0x0f) as usize
    }
}

pub(crate) fn split_program(bytes: &[u8]) -> Result<(ShaderStage, Vec<Token>), TokenError> {
    if bytes.is_empty() {
        return Err(TokenError::Empty);
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(TokenError::UnalignedBytes(bytes.len()));
    }
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    let stage = stage_of(words[0])?;

    let mut tokens = Vec::new();
    let mut at = 1;
    loop {
        let word = *words.get(at).ok_or(TokenError::MissingEnd)?;
        if word as u16 == END_TOKEN {
            return match words.len() - at - 1 {
                0 => Ok((stage, tokens)),
                trailing => Err(TokenError::TrailingWords(trailing)),
            };
        }
        let needed = operand_count(word) + 1;
        let have = words.len() - at;
        if have < needed {
            return Err(TokenError::Truncated {
                at_word: at,
                needed,
                have,
            });
        }
        if word as u16 != COMMENT_TOKEN {
            tokens.push(Token {
                at_word: at,
                word,
                operands: words[at + 1..at + needed].to_vec(),
            });
        }
        at += needed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    const PS_3_0: u32 = 0xffff_0300;
    const VS_3_0: u32 = 0xfffe_0300;

    #[test]
    fn splits_instructions_and_skips_comments() {
        // mov with two operand words, a one-word comment block, then end.
        let program = bytes(&[
            PS_3_0,
            0x0200_0001,
            0x800f_0800,
            0xa0e4_0000,
            0x0001_fffe,
            0xdead_beef,
            0x0000_ffff,
        ]);
        let (stage, tokens) = split_program(&program).unwrap();
        assert_eq!(stage, ShaderStage::Pixel);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].at_word, 1);
        assert_eq!(tokens[0].opcode(), 1);
        assert_eq!(tokens[0].operands, [0x800f_0800, 0xa0e4_0000]);
    }

    #[test]
    fn reads_the_stage_from_the_version() {
        let (stage, tokens) = split_program(&bytes(&[VS_3_0, 0xffff])).unwrap();
        assert_eq!(stage, ShaderStage::Vertex);
        assert!(tokens.is_empty());
    }

    #[test]
    fn refuses_a_malformed_binary() {
        assert_eq!(split_program(&[]).unwrap_err(), TokenError::Empty);
        assert_eq!(
            split_program(&[0; 5]).unwrap_err(),
            TokenError::UnalignedBytes(5)
        );
        assert_eq!(
            split_program(&bytes(&[0xffff_0200, 0xffff])).unwrap_err(),
            TokenError::UnsupportedVersion(0xffff_0200)
        );
        assert_eq!(
            split_program(&bytes(&[PS_3_0, 0x0000_0000])).unwrap_err(),
            TokenError::MissingEnd
        );
        assert_eq!(
            split_program(&bytes(&[PS_3_0, 0xffff, 0])).unwrap_err(),
            TokenError::TrailingWords(1)
        );
        assert_eq!(
            split_program(&bytes(&[PS_3_0, 0x0200_0001, 0x800f_0800])).unwrap_err(),
            TokenError::Truncated {
                at_word: 1,
                needed: 3,
                have: 2
            }
        );
    }
}
