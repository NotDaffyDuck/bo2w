//! What occupies the scene each frame: the local player's view, other
//! bodies of players, the shells in flight, scripted models and dynamic entities.

// The local player's view.
/// Clears per-match state when a match is torn down or swapped.
pub mod round_reset; // per-match state is cleared here
/// The killcam's camera.
pub mod death_replay; // the victim's view of their own death
/// Third-person, vehicle and death cameras.
pub mod other_view;
/// The view's kick and the camera taken from the presented snapshot.
pub mod eye_kick;
/// Keeps the held weapon in step with the player's state.
pub mod hold_sync; // weapon in hand
/// The first-person weapon and arms.
pub mod gun_present; // arms and gun
/// Camera shakes.
pub mod camera_shake;

// Other players and what they carry.
/// A dead player's body going limp.
pub mod ragdoll;
pub mod zm_ragdoll;
/// The gun's sway, bob and drift on the client.
pub mod t6_movement;
/// Sounds for bullet impacts and other players' bodies.
pub mod t6_sounds;
/// Corpses and the guns and bags on the ground.
pub mod t6_body;
/// Other players' bodies.
pub mod peer_body; // other clients

// The world's moving things.
/// Models placed by scripts.
pub mod script_prop;
/// Projectiles in flight and scripted models.
pub mod missile;
/// Moving platforms carry what stands on them.
pub mod platform_carry;
/// Dropped items.
pub mod item;
/// Physics for dynamic entities.
pub mod debris_phys;
/// Dynamic entities (debris, props) and their lighting links.
pub mod shard_piece;
// What the rest of the crate imports from this module.
pub use self::eye_kick::{GunOffset, QueuedHurtFlinch, sync_camera_from_shown};
pub use self::script_prop::{DrawFocus, ScriptPropDrawStage, ScriptPropSkinStage};
pub use self::peer_body::{OtherBodyEffectTags, OtherPlayerBody};
pub use self::gun_present::{FpvGeometrySet, GunMountAnchor, GunMountStage, ActiveGunMeshes};
pub use self::gun_present::{OwnSpawnPrimed, spawn_viewmodel, populate_gun_scene};
pub use self::gun_present::{stamp_matrix, tick_viewmodel};
pub use self::debris_phys::{PropPhysicsShape, DebrisPhysLevel};
pub use self::shard_piece::DebrisRoomBits;
