//! Usage codes: which shader input an element is bound to.

pub const VERTEX_ROLE_POSITION: u8 = 0;
pub const VERTEX_ROLE_NORMAL: u8 = 3;
pub const VERTEX_ROLE_UV: u8 = 5;
pub const VERTEX_USAGE_TINT: u8 = 10;
pub const VERTEX_ROLE_DEPTH: u8 = 12;

/// A usage code together with its index; `uv_semantic(2)` is the third set of
/// texture coordinates. Ordered by usage first, then index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VertexInputRole {
    pub usage: u8,
    pub semantic_ordinal: u8,
}

impl VertexInputRole {
    pub const fn new(usage: u8, semantic_ordinal: u8) -> Self {
        Self { usage, semantic_ordinal }
    }

    pub const fn position(semantic_ordinal: u8) -> Self {
        Self::new(VERTEX_ROLE_POSITION, semantic_ordinal)
    }

    pub const fn uv_semantic(semantic_ordinal: u8) -> Self {
        Self::new(VERTEX_ROLE_UV, semantic_ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthands_match_new() {
        assert_eq!(VertexInputRole::position(1), VertexInputRole::new(VERTEX_ROLE_POSITION, 1));
        assert_eq!(VertexInputRole::uv_semantic(3), VertexInputRole::new(VERTEX_ROLE_UV, 3));
    }

    #[test]
    fn order_is_usage_then_index() {
        assert!(VertexInputRole::position(7) < VertexInputRole::uv_semantic(0));
        assert!(VertexInputRole::uv_semantic(0) < VertexInputRole::uv_semantic(1));
    }
}
