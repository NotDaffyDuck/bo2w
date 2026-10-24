//! BO2's model collision tree rows. `XSurfaceCollisionNode` is 16 bytes: an
//! AABB of `u16` mins at 0 and maxs at 6, `childBeginIndex` at 12 and
//! `childCount` at 14. `XSurfaceCollisionLeaf` is 2 bytes, `triangleBeginIndex`.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XSurfaceCollisionNode {
    pub mins: [u16; 3],

    pub maxs: [u16; 3],

    pub child_begin_index: u16,

    pub child_count: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XSurfaceCollisionLeaf {
    pub triangle_begin_index: u16,
}
