//! Animation-side adapters: `render_anim`'s modules under the names the front end uses,
//! and the loose-piece (dyn ent) systems.

pub use render_anim::motion::{
    body_cull, puppet_pose, fpv, gun_host, gun_pose, pose_kinds, scene_feed,
    kick_state, gun_sway, gun_rig_ctl, figure_pose,
};
pub use render_anim::occupancy::{
    debris_phys, gun_present, hold_sync, item, round_reset, missile, peer_body, script_prop,
    other_view, eye_kick,
};

pub mod debris;
pub mod debris_piece;
pub mod free_body_brush;
pub mod free_body_wake;
