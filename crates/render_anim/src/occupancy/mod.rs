//! What occupies the scene each frame: the local player's view, other
//! players' bodies, projectiles, scripted models and dynamic entities.

// The local player's view.
/// Clears per-match state when a match is torn down or swapped.
pub mod match_reset; // per-match state is cleared here
/// The killcam's camera.
pub mod killcam; // the victim's view of their own death
/// Third-person, vehicle and death cameras.
pub mod third_person;
/// The view's kick and the camera taken from the presented snapshot.
pub mod view_kick;
/// Keeps the held weapon in step with the player's state.
pub mod held_sync; // weapon in hand
/// The first-person weapon and arms.
pub mod fpv_present; // arms and gun
/// Camera shakes.
pub mod camera_shake;

// Other players and what they carry.
/// A dead player's body going limp.
pub mod ragdoll;
/// The gun's sway, bob and drift on the client.
pub mod t6_movement;
/// Sounds for bullet impacts and other players' bodies.
pub mod t6_sounds;
/// Corpses and the guns and bags on the ground.
pub mod t6_body;
/// Other players' bodies.
pub mod remote_body; // other clients

// The world's moving things.
/// Models placed by scripts.
pub mod script_model;
/// Projectiles in flight and scripted models.
pub mod missile;
/// Dropped items.
pub mod item;
/// Physics for dynamic entities.
pub mod dyn_ent_phys;
/// Dynamic entities (debris, props) and their lighting links.
pub mod dyn_ent;
// What the rest of the crate imports from this module.
pub use self::view_kick::{GunOffset, PendingViewHurt, sync_camera_from_presented};
pub use self::script_model::{RenderFocus, ScriptModelDrawSet, ScriptModelSkinSet};
pub use self::remote_body::{RemoteFxBolts, RemotePlayer};
pub use self::fpv_present::{FpvGeometrySet, FpvPlacementRoot, FpvPlacementSet, SessionViewmodel};
pub use self::fpv_present::{LocalSpawnArmed, spawn_pending_fpv, occupy_fpv_scene};
pub use self::fpv_present::{stamp_fpv_placement_matrix, tick_fpv_viewmodel};
pub use self::dyn_ent_phys::{DynEntPhysClip, DynEntPhysWorld};
pub use self::dyn_ent::DynEntCellBits;
