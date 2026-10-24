//! Declaration usages: the meaning an element carries into the shader.

pub const D3DDECLUSAGE_POSITION: u8 = 0;
pub const D3DDECLUSAGE_NORMAL: u8 = 3;
pub const D3DDECLUSAGE_TEXCOORD: u8 = 5;
pub const D3DDECLUSAGE_COLOR: u8 = 10;
pub const D3DDECLUSAGE_DEPTH: u8 = 12;

/// A usage plus its index: `texcoord(2)` is the third texture coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Semantic {
    pub usage: u8,
    pub usage_index: u8,
}

impl Semantic {
    pub const fn new(usage: u8, usage_index: u8) -> Self {
        Self { usage, usage_index }
    }

    pub const fn position(usage_index: u8) -> Self {
        Self::new(D3DDECLUSAGE_POSITION, usage_index)
    }

    pub const fn texcoord(usage_index: u8) -> Self {
        Self::new(D3DDECLUSAGE_TEXCOORD, usage_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_fill_the_usage() {
        assert_eq!(Semantic::position(1), Semantic::new(D3DDECLUSAGE_POSITION, 1));
        assert_eq!(Semantic::texcoord(3), Semantic::new(D3DDECLUSAGE_TEXCOORD, 3));
    }

    #[test]
    fn semantics_sort_by_usage_then_index() {
        assert!(Semantic::position(7) < Semantic::texcoord(0));
        assert!(Semantic::texcoord(0) < Semantic::texcoord(1));
    }
}
