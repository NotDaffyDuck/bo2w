//! The per-frame viewmodel tick: decide whether the equipped weapon is still
//! the right one, gather what the server says about the gun, and ask the pose
//! generator for this frame's bones.

use assets::ReadiedFpvMeshes;
use bevy::prelude::*;
use frame::EyeSubject;
use net::{HostViewClient, ShownWorldFrame};
use playerstate_t6::game::{AvatarState, get_viewmodel_weapon_index, ledge_climb_bits, movement_state_bits};
use weapon_t6::ammo_table::{clip_table_key, get_clip_for_hand};

use super::gun_status::{
    GunMeshHandleSet, ActiveGunMeshes, same_compositions, same_material_catalog,
};
use super::weapon::{LOCAL_CLIENT, drawgun_flag, fpv_rocket_should_attach, fpv_viewmodel_weapon};
use crate::motion::fpv::{
    ServerGunCues, ServerTruthSnapshot, ViewmodelShowState, is_predicted_fire_weap_animation,
    own_shot_signature,
};
use crate::motion::gun_host::{
    AttachTagGoals, GunPoseInputs, HoldSettledLife, ViewmodelPoseVariety, GunPoseOutput, GunPoseDecline,
    FpvShownCursor, QueuedGunNotes, WaitingFpvSpawn, QueuedGunSpawn,
    generate_fpv_posture,
};
use crate::motion::fpv_prepared::{FpvWeaponSlot, FpvWeaponTable, PreparedFpv};
use crate::occupancy::other_view::shown_in_third_person;
use crate::occupancy::eye_kick::CameraRecoilState;

/// A melee strike is in progress when the weapon state is one of these.
fn is_striking(ps: &AvatarState) -> bool {
    matches!(
        ps.weapon_state_t6,
        weapon_t6::state::MELEE_INIT | weapon_t6::state::MELEE_FIRE
    )
}

/// Rounds in the clip of `hand` for the weapon being shown, if the table knows
/// the weapon.
fn magazine_rounds(ps: &AvatarState, table: Option<&FpvWeaponTable>, hand: u8) -> Option<i32> {
    let table = table?;
    let shown = fpv_viewmodel_weapon(ps, table);
    let facts = table.info_of(shown)?;
    let key = clip_table_key(facts.motion_clip_slot, shown);
    Some(get_clip_for_hand(&ps.magazine_rows, key, hand))
}

/// Checks whether the player's held weapon still matches the spawned
/// viewmodel. When it does not, the session is retargeted if the new weapon
/// shares the old one's models, otherwise a respawn is queued. Returns the
/// pose to publish instead of a tick when the tick must stop here.
fn follow_held_weapon(
    ps: &AvatarState,
    table: Option<&FpvWeaponTable>,
    session: &mut GunMeshHandleSet,
    catalog_id: u64,
    respawn_queue: &mut WaitingFpvSpawn,
) -> Option<ViewmodelPoseVariety> {
    let weapon = table.map_or(get_viewmodel_weapon_index(ps), |t| {
        fpv_viewmodel_weapon(ps, t)
    });
    let unchanged =
        weapon == 0 || (weapon == session.gun_catalog_no && ps.weapon_primary == session.parent_weapon);
    if unchanged {
        return None;
    }
    let table = match table {
        Some(table) => table,
        None => return Some(ViewmodelPoseVariety::Refuse(GunPoseDecline::TableAbsent)),
    };
    let gun_index = match table.gun_index(weapon) {
        Some(index) => index,
        None => return Some(ViewmodelPoseVariety::Hide),
    };
    let new_gun_model = gun_index != session.fpv.gun_index;
    // Same gun model: if the assemblies are the very same objects the swap is
    // free, else the viewmodel has to be rebuilt.
    let swap_is_free = !new_gun_model
        && matches!(
            table.gun_slot(weapon, ps.weapon_primary, session.axes),
            FpvWeaponSlot::Ready(next)
                if same_compositions(&next.assemblies, &session.view.assemblies)
        );
    if swap_is_free {
        (session.gun_catalog_no, session.parent_weapon) = (weapon, ps.weapon_primary);
        return None;
    }
    respawn_queue.0 = Some(QueuedGunSpawn {
        gun_index,
        catalog_id,
        gun_catalog_no: weapon,
        parent_weapon: ps.weapon_primary,
    });
    if new_gun_model {
        diag::info!(
            ViewModelPerspective,
            "fpv: weapon id -> {weapon}; the new gun needs its own model, respawning"
        );
    }
    Some(ViewmodelPoseVariety::Hide)
}

/// Why a frame stops before any posing, and what to publish instead.
enum EarlyStop {
    /// Hide the gun and throw the spawned viewmodel away.
    Retire,
    Show(ViewmodelPoseVariety),
}

/// The checks that end a tick before the pose is generated, in the order
/// they apply.
fn early_stop(
    session: &GunMeshHandleSet,
    materials_in: Option<&render_scene::StitchMaterials>,
    meshes: Option<&ReadiedFpvMeshes>,
    third_person: bool,
) -> Option<EarlyStop> {
    if !same_material_catalog(&session.material_catalog, materials_in) {
        diag::warn!(
            ViewModelPerspective,
            "fpv: material catalog changed; retiring stale rig and bindings"
        );
        return Some(EarlyStop::Retire);
    }
    let catalog_matches = meshes.is_some_and(|m| m.0.identity() == session.catalog_id);
    if !catalog_matches {
        return Some(EarlyStop::Retire);
    }
    third_person.then_some(EarlyStop::Show(ViewmodelPoseVariety::Hide))
}

/// The server-side facts the pose generator samples this frame, and whether a
/// shot fired locally this frame (before the server confirms it). Also stages
/// the viewmodel player's frame in the cursor.
fn gather_authority(
    presented: &ShownWorldFrame,
    local: &HostViewClient,
    table: Option<&FpvWeaponTable>,
    cursor: &mut ViewmodelShowState,
    bo2_table: Option<&sim::gun_view_facts::T6VmTableRes>,
) -> (Option<ServerTruthSnapshot>, bool) {
    let snap_in = match presented.snapshot() {
        Some(snap_in) => snap_in,
        None => return (None, false),
    };
    let pstate = presented.player(local.0);
    let meta = snap_in.meta.for_client(local.0);

    // The gun and its state come from the same player state: the predicted
    // one while there is a prediction.
    let t6_view = presented
        .local_t6_view()
        .or_else(|| meta.and_then(|m| m.t6_view));
    cursor.bo2.frame = match (pstate, t6_view) {
        (Some(pstate), Some(t6)) => {
            if let Some(res) = bo2_table {
                if !std::sync::Arc::ptr_eq(&cursor.bo2.table, &res.0) {
                    cursor.bo2.table = std::sync::Arc::clone(&res.0);
                }
            }
            let rounds = magazine_rounds(pstate, table, 0);
            Some(weapon_t6::viewmodel::VmFrame {
                weapon: pstate.weapon,
                model: table.map_or(pstate.weapon, |t| fpv_viewmodel_weapon(pstate, t)),
                gun_clip: t6.weap_anim,
                weap_anim_left: t6.weap_anim_left,
                gun_phase: t6.weaponstate,
                weaponstate_left: t6.weaponstate_left,
                gun_timer_ms: t6.weapon_time,
                weapon_time_left: t6.weapon_time_left,
                intro_shots_fired: t6.intro_shots_fired,
                ammo_in_clip: rounds.unwrap_or(0),
                ammo_in_clip_left: magazine_rounds(pstate, table, 1).unwrap_or(0),
                gun_raise_share: pstate.weapon_raise_fraction,
                perks: meta.map_or(0, |m| m.t6_perks),
                movement_mode: pstate.movement_mode,
                stride_velocity: pstate.velocity[0].hypot(pstate.velocity[1]),
                ps_speed: pstate.speed as f32,
                off_hand_weapon: u32::try_from(pstate.secondary_hand_slot).unwrap_or(0),
                // A low climb keeps the gun and times its animation to the climb.
                mantle_weapon_equipped: ledge_climb_bits::bo2_weapon_up(pstate),
                mantle_duration_ms: ledge_climb_bits::bo2_duration_ms(pstate.mantle_state.flags),
                mantle_do_anim: ledge_climb_bits::bo2_weapon_up(pstate)
                    && pstate.mantle_state.flags & ledge_climb_bits::BO2_WEAPON_ANIM != 0,
                ..Default::default()
            })
        }
        _ => None,
    };

    let locally_guessed_shot = pstate.is_some_and(|pstate| {
        let life = meta.map_or(0, |m| m.life_sequence.0);
        let shot = own_shot_signature(life, 0, pstate.shots_fired, pstate.weap_anim_t6);
        // Observe unconditionally: the edge detector needs every frame.
        let is_new = cursor.local_shot.note_shot(shot);
        is_new && is_predicted_fire_weap_animation(pstate.weap_anim_t6)
    });
    let host_cues = presented.first_person_hints(local.0);
    let sample = ServerTruthSnapshot {
        gun_state: pstate.map_or(0, |p| p.weapon_state_t6),
        advance: snap_in.tick.0,
        cues: ServerGunCues {
            created: host_cues.spawned,
            trigger_let_go: host_cues.trigger_released,
            fire_confirmed: false,
        },
        is_sprinting: pstate.is_some_and(|p| p.movement_state_bits & movement_state_bits::SPRINTING != 0),
        frac_ads: pstate.map_or(0.0, |p| p.weapon_raise_fraction),
        weap_animation: pstate.map_or(0, |p| p.weap_anim_t6),
        weap_animation_secondary: pstate.map_or(0, |p| p.weap_anim_left_t6),
        previous_gun_hand: pstate.map_or(0, |p| p.previous_gun_hand),
        specialties: pstate.map_or([0; 2], |p| p.perks),
        magazine_rounds: pstate.and_then(|pstate| magazine_rounds(pstate, table, 0)),
        left_clip_ammo: pstate.and_then(|pstate| magazine_rounds(pstate, table, 1)),
    };
    (Some(sample), locally_guessed_shot)
}

/// Starts a frame with nothing published: no notetracks, no attach points,
/// no pose.
fn clear_frame_outputs(
    notes: &mut QueuedGunNotes,
    bolts_in: &mut AttachTagGoals,
    out: &mut GunPoseOutput,
) {
    *out = GunPoseOutput::default();
    bolts_in.clear();
    notes.labels.clear();
    notes.weap = 0;
}

/// Records the notetracks a posed frame fired and where effects attach.
fn publish_frame_extras(
    posed: &mut ViewmodelPoseVariety,
    gun_catalog_no: u32,
    notes: &mut QueuedGunNotes,
    bolts_v: &mut AttachTagGoals,
) {
    let ViewmodelPoseVariety::Posed(frame) = posed else {
        return;
    };
    if gun_catalog_no != 0 && !frame.note_list.is_empty() {
        notes.weap = gun_catalog_no;
        notes.labels.clone_from(&frame.note_list);
    }
    // Attachment points come from the bones alone; nothing waits for a vertex.
    for (hand, slot) in frame.poses.iter_mut().enumerate() {
        let Some(pose) = slot else { continue };
        bolts_v.put_pose(hand, core::mem::take(&mut pose.bolt));
    }
}

/// Runs the viewmodel for one frame. The result lands in [`GunPoseOutput`]:
/// hide, a refusal with its cause, or the posed bones. Also records which
/// notetracks fired and where effects should attach.
pub fn tick_viewmodel(
    time_in: Res<Time>,
    presented_in: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    mut present_cursor: ResMut<FpvShownCursor>,
    prepared: Res<PreparedFpv>,
    mesh_catalog: Option<Res<ReadiedFpvMeshes>>,
    mut vm_slot: ResMut<ActiveGunMeshes>,
    mats: Option<Res<render_scene::StitchMaterials>>,
    mut held_settled: ResMut<HoldSettledLife>,
    mut spawn_queue: ResMut<WaitingFpvSpawn>,
    mut out: ResMut<GunPoseOutput>,
    mut note_out: ResMut<QueuedGunNotes>,
    mut bolt_out: ResMut<AttachTagGoals>,
    eye_kick: Option<Res<CameraRecoilState>>,
    view: Res<EyeSubject>,
    bo2_table: Option<Res<sim::gun_view_facts::T6VmTableRes>>,
) {
    let table = prepared.table().map(|t| &**t);
    clear_frame_outputs(&mut note_out, &mut bolt_out, &mut out);
    LOCAL_CLIENT.store(local.0.0, std::sync::atomic::Ordering::Relaxed);

    let person_third = shown_in_third_person(&presented_in, local.0, view.replaying_final_death());
    let shown_ps = presented_in.first_person_weapon_holder(local.0);
    if let Some(ps) = shown_ps.filter(|_| !person_third) {
        let into_ads = eye_kick.as_ref().map_or(true, |k| k.snap_to_sights);
        out.viewmodel_pass_flag = drawgun_flag(ps, table, into_ads);
    }

    let Some(session) = vm_slot.0.as_mut() else {
        out.kind = ViewmodelPoseVariety::Hide;
        return;
    };
    let stop = early_stop(
        session,
        mats.as_deref(),
        mesh_catalog.as_deref(),
        person_third,
    );
    match stop {
        Some(EarlyStop::Retire) => {
            // Drop the spawned viewmodel and everything queued for it.
            vm_slot.0 = None;
            held_settled.0 = None;
            spawn_queue.0 = None;
            out.kind = ViewmodelPoseVariety::Hide;
            return;
        }
        Some(EarlyStop::Show(kind)) => {
            out.kind = kind;
            return;
        }
        None => {}
    }
    let Some(meshes) = mesh_catalog.as_ref() else {
        out.kind = ViewmodelPoseVariety::Refuse(GunPoseDecline::TableAbsent);
        return;
    };
    // bo2zm: the script changed his hands (Mob's Afterlife): the same weapon's view from the
    // new table.
    if let Some(table) = table
        && let FpvWeaponSlot::Ready(next) =
            table.gun_slot(session.gun_catalog_no, session.parent_weapon, session.axes)
        && next.script_hands != session.view.script_hands
        && let Some(gun_index) = table.gun_index(session.gun_catalog_no)
    {
        diag::info!(ViewModelPerspective, "fpv: script hands changed; re-spawn the view");
        spawn_queue.0 = Some(QueuedGunSpawn {
            gun_index,
            catalog_id: meshes.0.identity(),
            gun_catalog_no: session.gun_catalog_no,
            parent_weapon: session.parent_weapon,
        });
        out.kind = ViewmodelPoseVariety::Hide;
        return;
    }

    if let Some(kind) = shown_ps.and_then(|ps| {
        follow_held_weapon(ps, table, session, meshes.0.identity(), &mut spawn_queue)
    }) {
        out.kind = kind;
        return;
    }

    let (probe, locally_guessed_shot) = gather_authority(
        &presented_in,
        &local,
        table,
        &mut present_cursor.0,
        bo2_table.as_deref(),
    );
    let rocket_visible = table
        .is_some_and(|t| fpv_rocket_should_attach(t, session.gun_catalog_no, presented_in.player(local.0)));
    let dual = shown_ps.is_some_and(|ps| ps.previous_gun_hand == 1);
    let akimbo_shift = shown_ps
        .filter(|_| dual)
        .and_then(|ps| table?.info_of(fpv_viewmodel_weapon(ps, table?)))
        .map(|facts| facts.twin_grip_viewmodel_shift);
    let mantle_camera = shown_ps.and_then(|ps| {
        (ps.mantle_state.flags & ledge_climb_bits::BO2_WEAPON_ANIM != 0)
            .then(|| ledge_climb_bits::bo2_progress(ps))
            .flatten()
    });

    let gun_catalog_no = session.gun_catalog_no;
    let GunMeshHandleSet {
        fpv: equipped,
        active_rig,
        view: equipped_view,
        ..
    } = session;
    let mut kind = generate_fpv_posture(GunPoseInputs {
        dt: time_in.delta_secs(),
        equipped,
        rigs: &equipped_view.rigs,
        active: active_rig,
        cursor: &mut present_cursor.0,
        rocket: rocket_visible,
        melee: shown_ps.is_some_and(is_striking),
        // A sight with a stripped aimed model shows it from the fraction the
        // reticle fades in at.
        ads: shown_ps.is_some_and(|ps| ps.weapon_raise_fraction >= 0.75),
        sample: probe,
        locally_guessed_shot,
        akimbo: dual,
        akimbo_shift,
        mantle_camera,
    });
    publish_frame_extras(&mut kind, gun_catalog_no, &mut note_out, &mut bolt_out);
    out.kind = kind;
}
