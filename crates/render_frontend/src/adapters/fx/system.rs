//! The combat effects' systems: when each runs in the frame, and the modules that do the work.
//!
//! A frame of effects is ticked in two halves around the pose step, generated into one
//! transaction, and committed into the draw plans the renderer consumes. Effects that start from
//! an event (a shot, a blast, a bullet hit) are observers that queue into the same host.

mod blast_fx;
mod commit;
mod dump;
mod glass_cues;
mod gun_fx;
mod hit_fx;
mod lifecycle;
mod mark_plan;
mod mesh_bind;
mod missile_fx;
mod model_plan;
mod play_fx;
mod scene_access;
mod script_fx;
mod sprite_runs;
mod stage;
mod t6_marks;
mod t6_mesh;
mod ticks;
mod transaction;
mod viewpoint;
mod weapon_audio;

use bevy::prelude::*;
use net::ClientSet;
use render_fx::system::stamp_fx_camera_origin;

use crate::assemble::drawsurf::tess::glass::{apply_glass_host, draw_t6_glass_panes};

use transaction::{FxFrameTransaction, FxPresentEnv, generate_fx_transaction};
use scene_access::FxSceneAccess;
use t6_marks::{t6_fx_filtered_out, t6_fx_material, take_t6_marks};
use t6_mesh::T6_FX_ELEM_DRAW_WITH_VIEWMODEL;

pub use t6_marks::T6MarkMesh;
pub use t6_mesh::{T6_DYN_LIGHTS, T6CloudDraw, T6DynLights, T6FxMesh};

pub(crate) use mesh_bind::{FxCodeMeshBind, FxPresentSkip};

/// The resources and messages the effects systems keep.
fn init_fx_state(app: &mut App) {
    app.add_message::<hit_fx::BulletHitFx>()
        .add_message::<render_anim::occupancy::t6_body::T6SlideDust>()
        .init_resource::<render_fx::FxModelStaging>()
        .init_resource::<T6FxMesh>()
        .init_resource::<T6DynLights>()
        .init_resource::<T6MarkMesh>()
        .init_resource::<crate::assemble::drawsurf::GlassTable>()
        .init_resource::<crate::assemble::drawsurf::T6GlassMesh>()
        .init_resource::<crate::assemble::drawsurf::T6GlassCues>()
        .init_resource::<super::gun_loops::GunLoopState>()
        .init_resource::<super::sensor_laser::SensorLasers>();
}

/// The frame's order: the map's windows and non-pose-dependent effects, then the pose-dependent
/// remainder, then the transaction that publishes it all.
fn schedule_fx_frame(app: &mut App) {
    use ::frame::WorkerCmdSet as Stage;

    app.add_systems(Update, lifecycle::latch_authority_load_hold.in_set(ClientSet::Load))
        .add_systems(
            Update,
            (
                lifecycle::boot_createfx_oneshots,
                script_fx::sync_script_fx,
                apply_glass_host,
                ticks::tick_fx_non_dependent_update,
            )
                .chain()
                .after(stamp_fx_camera_origin)
                .in_set(Stage::FxNonDependent),
        )
        // The map's windows, once this frame's glass states are in and the shards have moved.
        .add_systems(
            Update,
            draw_t6_glass_panes
                .after(apply_glass_host)
                .after(ticks::tick_fx_non_dependent_update),
        )
        .add_systems(
            Update,
            ticks::tick_fx_remaining_update
                .after(Stage::SkinModel)
                .in_set(Stage::FxRemaining),
        )
        .add_systems(
            crate::assemble::StaticSunAndFx,
            generate_fx_transaction.pipe(commit::commit_fx_transaction),
        );
}

/// Effects that play from this frame's poses: bullet hits at the muzzle tags and a sliding
/// player's dust. Both wait for the bodies and weapons to be posed.
fn schedule_posed_effects(app: &mut App) {
    use ::frame::WorkerCmdSet as Stage;
    use render_anim::occupancy::{fpv_present::publish_fpv_dobj_pose, t6_body::T6BodyPosesPublished};

    app.add_systems(
        Update,
        (hit_fx::play_queued_hits, hit_fx::play_predicted_pellets)
            .chain()
            .after(publish_fpv_dobj_pose)
            .after(Stage::SkinModel)
            .before(Stage::FxNonDependent)
            .in_set(ClientSet::Present),
    )
    .add_systems(
        Update,
        lifecycle::play_t6_slide_dust
            .after(T6BodyPosesPublished)
            .in_set(ClientSet::Present),
    );
}

/// Effects that start from an event rather than from the frame.
fn observe_fx_events(app: &mut App) {
    app.add_systems(
        Update,
        (missile_fx::tick_missile_present_state, super::gun_loops::tick).in_set(ClientSet::Effects),
    )
    .add_observer(super::gun_loops::note_fire)
    .add_observer(super::sensor_laser::note)
    .add_observer(super::bolt_impact::play)
    .add_observer(super::melee_glass::play)
    .add_observer(gun_fx::fire_weapon)
    .add_observer(gun_fx::eject_brass)
    .add_observer(blast_fx::explosion)
    .add_observer(blast_fx::grenade_bounce)
    .add_observer(blast_fx::stop_killcam_explosion_fx)
    .add_observer(play_fx::play_fx)
    .add_observer(hit_fx::queue_bullet_hit)
    .add_observer(hit_fx::knife_blood);
}

pub(crate) fn register_combat_fx_systems(app: &mut App) {
    init_fx_state(app);
    schedule_fx_frame(app);
    schedule_posed_effects(app);
    observe_fx_events(app);
}
