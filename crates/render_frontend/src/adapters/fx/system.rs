//! The combat effects' systems: when each runs in the frame, and the modules that do the work.
//!
//! A frame of effects is ticked in two halves around the pose step, generated into one
//! transaction, and committed into the draw plans the renderer consumes. Effects that start from
//! an event (a shot, a blast, a bullet hit) are observers that queue into the same host.

mod blast_fx;
mod commit;
mod dump;
mod ghost_hands;
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
mod tx;
mod viewpoint;
mod weapon_audio;

use frame::JobQueueStage;
use bevy::app::Update;
use bevy::prelude::*;
use net::ClientStep;
use render_anim::occupancy::{gun_present::publish_dobj_pose, t6_body::T6BodyPosesPublished};
use render_fx::system::record_effects_eye_position;

use crate::assemble::StaticSunAndFx;
use crate::assemble::draw_items::mesh_build::glass_piece::{apply_glass_host, draw_t6_glass_panes};

use scene_access::EffectStageAccess;
use t6_marks::{t6_fx_filtered_out, t6_fx_material, take_t6_marks};
use t6_mesh::T6_FX_ELEM_DRAW_WITH_VIEWMODEL;
use tx::{EffectFrameTransaction, EffectPresentEnv, open_fx_transaction};

pub use t6_marks::T6MarkMesh;
pub use t6_mesh::{T6_DYN_LIGHTS, T6CloudDraw, T6DynLights, T6FxMesh};

pub(crate) use mesh_bind::{EffectEngineMeshBind, EffectPresentSkip};

/// Registers every combat-effects system, resource and observer on the client app.
pub(crate) fn install_combat_fx(app: &mut App) {
    keep_state(app);
    order_frame(app);
    order_after_poses(app);
    hook_events(app);
}

/// What the effects keep between frames: staged models, the mesh buffers they fill, the glass
/// tables and the looping gun and laser sounds.
fn keep_state(app: &mut App) {
    app.add_message::<hit_fx::BulletHitFx>();
    app.add_message::<render_anim::occupancy::t6_body::T6SlideDust>();

    app.init_resource::<render_fx::FxModelStagingPlan>();
    app.init_resource::<T6FxMesh>();
    app.init_resource::<T6DynLights>();
    app.init_resource::<T6MarkMesh>();

    app.init_resource::<crate::assemble::draw_items::GlassTable>();
    app.init_resource::<crate::assemble::draw_items::T6GlassMesh>();
    app.init_resource::<crate::assemble::draw_items::T6GlassCues>();

    app.init_resource::<super::gun_loops::GunLoopState>();
    app.init_resource::<super::sensor_laser::SensorLasers>();
}

/// One frame, in order: the map-side effects that need no pose, the window panes once the glass
/// has moved, the effects that wait for the skinned models, then the transaction that publishes
/// all of it into the draw plans.
fn order_frame(client: &mut App) {
    client.add_systems(Update, lifecycle::latch_authority_load_hold.in_set(ClientStep::Load));

    let pose_free = (
        lifecycle::boot_createfx_oneshots,
        script_fx::sync_script_fx,
        apply_glass_host,
        ticks::tick_fx_non_dependent_update,
    )
        .chain();
    client.add_systems(
        Update,
        pose_free
            .after(record_effects_eye_position)
            .in_set(JobQueueStage::FxNonDependent),
    );

    client.add_systems(
        Update,
        draw_t6_glass_panes
            .after(apply_glass_host)
            .after(ticks::tick_fx_non_dependent_update),
    );

    client.add_systems(
        Update,
        ticks::tick_fx_remaining_update
            .after(JobQueueStage::SkinModel)
            .in_set(JobQueueStage::FxRemaining),
    );

    client.add_systems(
        StaticSunAndFx,
        open_fx_transaction.pipe(commit::commit_fx_transaction),
    );
}

/// Effects drawn from this frame's finished poses: bullet hits at the muzzle tags, and the dust
/// kicked up by a sliding body.
fn order_after_poses(app: &mut App) {
    // bo2zm: Mob's Afterlife ghost hands ride the viewmodel's wrists too.
    let hits = (
        hit_fx::play_queued_hits,
        hit_fx::play_predicted_pellets,
        ghost_hands::present_ghost_hands_fx,
    )
        .chain();
    app.add_systems(
        Update,
        hits.after(publish_dobj_pose)
            .after(JobQueueStage::SkinModel)
            .before(JobQueueStage::FxNonDependent)
            .in_set(ClientStep::Present),
    );

    app.add_systems(
        Update,
        lifecycle::play_t6_slide_dust
            .after(T6BodyPosesPublished)
            .in_set(ClientStep::Present),
    );
}

/// Effects started by an event instead of by the frame, plus the two per-frame ticks that keep
/// their loops going.
fn hook_events(client: &mut App) {
    let loop_ticks = (missile_fx::tick_missile_present_state, super::gun_loops::tick);
    client.add_systems(Update, loop_ticks.in_set(ClientStep::Effects));

    client.add_observer(super::gun_loops::note_fire);
    client.add_observer(super::sensor_laser::note);
    client.add_observer(super::bolt_impact::play);
    client.add_observer(super::melee_glass::play);

    client.add_observer(gun_fx::fire_weapon);
    client.add_observer(gun_fx::eject_brass);
    client.add_observer(blast_fx::explosion);
    client.add_observer(blast_fx::grenade_bounce);
    client.add_observer(blast_fx::stop_killcam_explosion_fx);
    client.add_observer(play_fx::play_fx);
    client.add_observer(hit_fx::queue_bullet_hit);
    client.add_observer(hit_fx::knife_blood);
}
