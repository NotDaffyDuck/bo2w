//! The host's scene lists, re-exported from `render_scene`.

pub use render_scene::{
    AddBModelArgs, AddBModelPose, AddDObjArgs, AddDObjPose, GfxScene, GfxSceneBrush, GfxSceneDobj,
    GfxSceneModel, SceneAddError, SceneAddKind, SceneEntSkinnedSurfs,
};
pub use render_scene::{
    SCENE_BRUSH_CAP, SCENE_DOBJ_CAP, SCENE_GFX_ENT_CAP, SCENE_INDEX_EMPTY, SCENE_MODEL_CAP,
    SCENE_VIEWMODEL_ENTNUM, SCENE_VIEWMODEL_FX_FLAGS, SCENE_VIEWMODEL_LEFT_ENTNUM,
};
pub use render_scene::{pack_scene_info, scene_info_entnum};
