//! The first-person view of a weapon after its models are joined into one
//! skeleton: which model owns which bones, and where the effect tags are.

use std::sync::Arc;

use xmodel_runtime::{DObj, DObjError};

use crate::FpvMeshIndex;

/// What a joined model is to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpvPartRole {
    Hands,
    Gun,
    Attachment,
    Rocket,
    Knife,
}

/// One model inside the joined skeleton.
#[derive(Clone, Debug)]
pub struct FpvAssemblyPart {
    pub model: FpvMeshIndex,
    pub role: FpvPartRole,
    /// Where this model's bones start in the joined skeleton.
    pub bone_base: usize,
    /// Surfaces hidden by an attachment, as part-bit words (guns only).
    pub hide: Option<[u32; 5]>,
}

/// The bones the gun's effects hang from; `None` where the joined skeleton
/// has no such bone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FpvAssemblyTags {
    pub flash: Option<u16>,
    pub flash_silenced: Option<u16>,
    pub brass: Option<u16>,
    pub knife: Option<u16>,
    pub laser: Option<u16>,
    pub tracker_screen: [Option<u16>; 3],
    pub tracker_light: Option<u16>,
    /// The flash and brass bones of a second gun that hangs on the arms.
    pub flash_left: Option<u16>,
    pub brass_left: Option<u16>,
    /// `tag_target`: where the client scripts put a light on the gun (the
    /// Hunter Killer's lock light).
    pub target: Option<u16>,
    /// `tag_hud`: the plane the Target Finder's diamonds are projected through.
    pub hud: Option<u16>,
}

/// A weapon view's models, joined.
#[derive(Debug)]
pub struct FpvAssembly {
    pub dobj: DObj,
    pub parts: Vec<FpvAssemblyPart>,
    pub view_bone: usize,
    pub camera_bone: Option<usize>,
    /// Bones of the arms and the gun together: the span a paired animation
    /// drives.
    pub paired_bones: usize,
    pub tags: FpvAssemblyTags,
    /// A second gun hangs on the arms (dual wield: one pair of arms, two
    /// guns, the left and right animations played together).
    pub arms_gun: bool,
}

#[derive(Debug)]
pub enum FpvAssemblyError {
    Catalog(&'static str),
    Skeleton(DObjError),
    NoTagView,
}

impl core::fmt::Display for FpvAssemblyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Catalog(what) => write!(f, "{what} missing from the first-person catalog"),
            Self::Skeleton(error) => write!(f, "combined skeleton: {error}"),
            Self::NoTagView => write!(f, "combined skeleton has no tag_view"),
        }
    }
}

/// What makes two views the same view, so one assembly can serve both.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FpvAssemblyKey {
    pub hands: FpvMeshIndex,
    pub gun: FpvMeshIndex,
    pub attachments: Vec<FpvMeshIndex>,
    /// Each attachment's root placement as raw bits (rotation, translation).
    pub attachment_binds: Vec<Option<([u32; 4], [u32; 3])>>,
    pub rocket: Option<FpvMeshIndex>,
    pub knife: Option<FpvMeshIndex>,
    pub hide_tags: Vec<String>,
}

/// The views one side of the player's weapon can show.
#[derive(Clone, Debug)]
pub struct FpvSideAssemblies {
    pub bare: Arc<FpvAssembly>,
    pub rocket: Option<Arc<FpvAssembly>>,
    pub melee: Option<Arc<FpvAssembly>>,
    /// The gun while aimed, when its sight swaps to a stripped model.
    pub ads: Option<Arc<FpvAssembly>>,
}

impl FpvSideAssemblies {
    /// The view with the rocket on when it is asked for and there is one.
    pub fn pick(&self, rocket: bool) -> &Arc<FpvAssembly> {
        match &self.rocket {
            Some(with) if rocket => with,
            _ => &self.bare,
        }
    }
}
