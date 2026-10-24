//! The per-frame viewmodel tick: decide whether the equipped weapon is still
//! the right one, gather what the server says about the gun, and ask the pose
//! generator for this frame's bones.

use assets::PreparedFpvMeshes;
use bevy::prelude::*;
use frame::ViewSubject;
use net::{LocalPresentClient, PresentedSnapshot};
use playerstate_t6::game::{PlayerState, get_viewmodel_weapon_index, mantle_flags, pm_flags};
use weapon_t6::ammo_table::{clip_table_key, get_clip_for_hand};

use super::session::{SessionFpvMeshesHandles, SessionViewmodel, same_compositions, same_material_catalog};
use super::weapon::{LOCAL_CLIENT, drawgun_flag, fpv_rocket_should_attach, fpv_viewmodel_weapon};
use crate::anim::fpv::{
    AuthorityFpvCues, FpvAuthoritySample, FpvPresentState, is_predicted_fire_weap_anim,
    local_shot_identity,
};
use crate::anim::fpv_host::{
    FpvBoltTargets, FpvGenerateArgs, FpvHeldSettled, FpvPoseKind, FpvPoseProduct, FpvPoseRefuse,
    FpvPresentCursor, PendingFpvNotetracks, PendingFpvSpawn, PendingFpvSpawnRequest,
    generate_fpv_pose,
};
use crate::anim::fpv_prepared::{FpvWeaponSlot, FpvWeaponTable, PreparedFpv};
use crate::occupancy::third_person::presented_is_third_person;
use crate::occupancy::view_kick::SessionViewKick;

/// A melee strike is in progress when the weapon state is one of these.
fn is_striking(ps: &PlayerState) -> bool {
    matches!(
        ps.weapon_state_t6,
        weapon_t6::state::MELEE_INIT | weapon_t6::state::MELEE_FIRE
    )
}

/// Rounds in the clip of `hand` for the weapon being shown, if the table knows
/// the weapon.
fn clip_ammo(ps: &PlayerState, table: Option<&FpvWeaponTable>, hand: u8) -> Option<i32> {
    let table = table?;
    let shown = fpv_viewmodel_weapon(ps, table);
    let facts = table.facts_of(shown)?;
    let key = clip_table_key(facts.clip_index, shown);
    Some(get_clip_for_hand(&ps.ammoclip, key, hand))
}

/// Checks whether the player's held weapon still matches the spawned
/// viewmodel. When it does not, the session is retargeted if the new weapon
/// shares the old one's models, otherwise a respawn is queued. Returns the
/// pose to publish instead of a tick when the tick must stop here.
fn follow_held_weapon(
    ps: &PlayerState,
    table: Option<&FpvWeaponTable>,
    session: &mut SessionFpvMeshesHandles,
    catalog_id: u64,
    pending: &mut PendingFpvSpawn,
) -> Option<FpvPoseKind> {
    let weapon = table.map_or(get_viewmodel_weapon_index(ps), |t| fpv_viewmodel_weapon(ps, t));
    let unchanged = weapon == 0
        || (weapon == session.weapon_id && ps.weapon_primary == session.parent_weapon);
    if unchanged {
        return None;
    }
    let Some(table) = table else {
        return Some(FpvPoseKind::Refuse(FpvPoseRefuse::CatalogMissing));
    };
    let Some(gun_index) = table.gun_index(weapon) else {
        return Some(FpvPoseKind::Hide);
    };
    let request = PendingFpvSpawnRequest {
        gun_index,
        catalog_id,
        weapon_id: weapon,
        parent_weapon: ps.weapon_primary,
    };
    if gun_index != session.fpv.gun_index {
        pending.0 = Some(request);
        diag::info!(
            Fpv,
            "fpv: weapon id → {weapon}; re-queue FPV for new gunXModel"
        );
        return Some(FpvPoseKind::Hide);
    }
    // Same gun model: if the assemblies are the very same objects the swap is
    // free, else the viewmodel has to be rebuilt.
    let shares_models = matches!(
        table.slot(weapon, ps.weapon_primary, session.axis),
        FpvWeaponSlot::Ready(next) if same_compositions(&next.assemblies, &session.view.assemblies)
    );
    if shares_models {
        session.weapon_id = weapon;
        session.parent_weapon = ps.weapon_primary;
        None
    } else {
        pending.0 = Some(request);
        Some(FpvPoseKind::Hide)
    }
}

/// The server-side facts the pose generator samples this frame, and whether a
/// shot fired locally this frame (before the server confirms it). Also stages
/// the viewmodel player's frame in the cursor.
fn gather_authority(
    presented: &PresentedSnapshot,
    local: &LocalPresentClient,
    table: Option<&FpvWeaponTable>,
    cursor: &mut FpvPresentState,
    bo2_table: Option<&sim::t6_viewmodel::T6VmTableRes>,
) -> (Option<FpvAuthoritySample>, bool) {
    let Some(snap) = presented.snapshot() else {
        return (None, false);
    };
    let ps = presented.player(local.0);
    let meta = snap.meta.for_client(local.0);

    // The gun and its state come from the same player state: the predicted
    // one while there is a prediction.
    let t6_view = presented
        .local_t6_view()
        .or_else(|| meta.and_then(|m| m.t6_view));
    cursor.bo2.frame = match (ps, t6_view) {
        (Some(ps), Some(t6)) => {
            if let Some(res) = bo2_table {
                if !std::sync::Arc::ptr_eq(&cursor.bo2.table, &res.0) {
                    cursor.bo2.table = std::sync::Arc::clone(&res.0);
                }
            }
            Some(weapon_t6::viewmodel::VmFrame {
                weapon: ps.weapon,
                model: table.map_or(ps.weapon, |t| fpv_viewmodel_weapon(ps, t)),
                weap_anim: t6.weap_anim,
                weap_anim_left: t6.weap_anim_left,
                weaponstate: t6.weaponstate,
                weaponstate_left: t6.weaponstate_left,
                weapon_time: t6.weapon_time,
                weapon_time_left: t6.weapon_time_left,
                intro_shots_fired: t6.intro_shots_fired,
                ammo_in_clip: clip_ammo(ps, table, 0).unwrap_or(0),
                ammo_in_clip_left: clip_ammo(ps, table, 1).unwrap_or(0),
                weapon_pos_frac: ps.f_weapon_pos_frac,
                perks: meta.map_or(0, |m| m.t6_perks),
                pm_type: ps.pm_type,
                move_speed: ps.velocity[0].hypot(ps.velocity[1]),
                ps_speed: ps.speed as f32,
                off_hand_weapon: u32::try_from(ps.off_hand_index).unwrap_or(0),
                // A low climb keeps the gun and times its animation to the climb.
                mantle_weapon_equipped: mantle_flags::bo2_weapon_up(ps),
                mantle_duration_ms: mantle_flags::bo2_duration_ms(ps.mantle_state.flags),
                mantle_do_anim: mantle_flags::bo2_weapon_up(ps)
                    && ps.mantle_state.flags & mantle_flags::BO2_WEAPON_ANIM != 0,
                ..Default::default()
            })
        }
        _ => None,
    };

    let predicted_fire = ps.is_some_and(|ps| {
        let life = meta.map_or(0, |m| m.life_sequence.0);
        let shot = local_shot_identity(life, 0, ps.weapon_shot_count, ps.weap_anim_t6);
        // Observe unconditionally: the edge detector needs every frame.
        let is_new = cursor.local_shot.observe(shot);
        is_new && is_predicted_fire_weap_anim(ps.weap_anim_t6)
    });
    let cues = presented.fpv_cues(local.0);
    let sample = FpvAuthoritySample {
        tick: snap.tick.0,
        weaponstate: ps.map_or(0, |p| p.weapon_state_t6),
        cues: AuthorityFpvCues {
            shot_accepted: false,
            attack_released: cues.attack_released,
            spawned: cues.spawned,
        },
        sprinting: ps.is_some_and(|p| p.pm_flags & pm_flags::SPRINTING != 0),
        ads_frac: ps.map_or(0.0, |p| p.f_weapon_pos_frac),
        weap_anim: ps.map_or(0, |p| p.weap_anim_t6),
        weap_anim_secondary: ps.map_or(0, |p| p.weap_anim_left_t6),
        last_weapon_hand: ps.map_or(0, |p| p.last_weapon_hand),
        perks0: ps.map_or(0, |p| p.perks[0]),
        clip_ammo: ps.and_then(|ps| clip_ammo(ps, table, 0)),
        left_clip_ammo: ps.and_then(|ps| clip_ammo(ps, table, 1)),
    };
    (Some(sample), predicted_fire)
}

/// Runs the viewmodel for one frame. The result lands in [`FpvPoseProduct`]:
/// hide, a refusal with its cause, or the posed bones. Also records which
/// notetracks fired and where effects should attach.
pub fn tick_fpv_viewmodel(
    time: Res<Time>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    mut cursor: ResMut<FpvPresentCursor>,
    prepared: Res<PreparedFpv>,
    fpv_meshes: Option<Res<PreparedFpvMeshes>>,
    mut session_vm: ResMut<SessionViewmodel>,
    tess: Option<Res<render_scene::TessMaterials>>,
    mut settled: ResMut<FpvHeldSettled>,
    mut pending: ResMut<PendingFpvSpawn>,
    mut product: ResMut<FpvPoseProduct>,
    mut pending_notes: ResMut<PendingFpvNotetracks>,
    mut bolts: ResMut<FpvBoltTargets>,
    kick: Option<Res<SessionViewKick>>,
    view: Res<ViewSubject>,
    bo2_table: Option<Res<sim::t6_viewmodel::T6VmTableRes>>,
) {
    let table = prepared.table().map(|t| &**t);
    pending_notes.weapon = 0;
    pending_notes.names.clear();
    bolts.clear();
    *product = FpvPoseProduct::default();
    LOCAL_CLIENT.store(local.0.0, std::sync::atomic::Ordering::Relaxed);

    let third_person = presented_is_third_person(&presented, local.0, view.in_killcam());
    let shown_ps = presented.viewweapon_player(local.0);
    if let Some(ps) = shown_ps.filter(|_| !third_person) {
        let into_ads = kick.as_ref().map_or(true, |k| k.b_position_to_ads);
        product.drawgun = drawgun_flag(ps, table, into_ads);
    }

    // Drops the spawned viewmodel and everything queued for it.
    let mut retire = |session_vm: &mut SessionViewmodel| {
        session_vm.0 = None;
        settled.0 = None;
        pending.0 = None;
    };
    let Some(session) = session_vm.0.as_mut() else {
        product.kind = FpvPoseKind::Hide;
        return;
    };
    if !same_material_catalog(&session.material_catalog, tess.as_deref()) {
        diag::warn!(
            Fpv,
            "fpv: material catalog changed; retiring stale rig and bindings"
        );
        retire(&mut session_vm);
        product.kind = FpvPoseKind::Hide;
        return;
    }
    let catalog_matches = fpv_meshes
        .as_ref()
        .is_some_and(|m| m.0.identity() == session.catalog_id);
    if !catalog_matches {
        retire(&mut session_vm);
        product.kind = FpvPoseKind::Hide;
        return;
    }
    if third_person {
        product.kind = FpvPoseKind::Hide;
        return;
    }
    let Some(meshes) = fpv_meshes.as_ref() else {
        product.kind = FpvPoseKind::Refuse(FpvPoseRefuse::CatalogMissing);
        return;
    };

    if let Some(ps) = shown_ps {
        let stop = follow_held_weapon(ps, table, session, meshes.0.identity(), &mut pending);
        if let Some(kind) = stop {
            product.kind = kind;
            return;
        }
    }

    let (sample, predicted_fire) = gather_authority(
        &presented,
        &local,
        table,
        &mut cursor.0,
        bo2_table.as_deref(),
    );
    let rocket_visible = table
        .is_some_and(|t| fpv_rocket_should_attach(t, session.weapon_id, presented.player(local.0)));
    let dual = shown_ps.is_some_and(|ps| ps.last_weapon_hand == 1);
    let dual_offset = shown_ps
        .filter(|_| dual)
        .and_then(|ps| table?.facts_of(fpv_viewmodel_weapon(ps, table?)))
        .map(|facts| facts.dual_wield_view_model_offset);
    let mantle_camera = shown_ps.and_then(|ps| {
        (ps.mantle_state.flags & mantle_flags::BO2_WEAPON_ANIM != 0)
            .then(|| mantle_flags::bo2_progress(ps))
            .flatten()
    });

    let weapon_id = session.weapon_id;
    let SessionFpvMeshesHandles {
        fpv: equipped,
        active_rig,
        view: equipped_view,
        ..
    } = session;
    let mut kind = generate_fpv_pose(FpvGenerateArgs {
        dt: time.delta_secs(),
        equipped,
        rigs: &equipped_view.rigs,
        active: active_rig,
        cursor: &mut cursor.0,
        rocket: rocket_visible,
        melee: shown_ps.is_some_and(is_striking),
        // A sight with a stripped aimed model shows it from the fraction the
        // reticle fades in at.
        ads: shown_ps.is_some_and(|ps| ps.f_weapon_pos_frac >= 0.75),
        sample,
        predicted_fire,
        dual,
        dual_offset,
        mantle_camera,
    });
    if let FpvPoseKind::Posed(frame) = &mut kind {
        if weapon_id != 0 && !frame.notetracks.is_empty() {
            pending_notes.weapon = weapon_id;
            pending_notes.names.clone_from(&frame.notetracks);
        }
        // Attachment points come from the bones alone; nothing waits for a vertex.
        for (hand, pose) in frame.poses.iter_mut().enumerate() {
            if let Some(pose) = pose.as_mut() {
                bolts.set_pose(hand, core::mem::take(&mut pose.bolt));
            }
        }
    }
    product.kind = kind;
}
