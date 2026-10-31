//! BO2's model collision tree rows. `XSurfaceCollisionNode` is 16 bytes: an
//! AABB of `u16` mins at 0 and maxs at 6, `childBeginIndex` at 12 and
//! `childCount` at 14. `XSurfaceCollisionLeaf` is 2 bytes, `triangleBeginIndex`.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollTreeNode {
    pub lo_corner: [u16; 3],

    pub hi_corner: [u16; 3],

    pub first_descendant_slot: u16,

    pub sub_element_total: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollTreeLeaf {
    pub first_tri: u16,
}
