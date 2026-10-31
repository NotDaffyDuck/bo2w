//! Single hits on a prop: a bullet, a thrown object, a script's `dodamage`.

use super::blast::{Blast, radius_pieces};
use super::publish::{Cause, publish};
use super::piece_damage::Strike;
use super::{
    DamageKind, MOD_BURNED, MOD_GRENADE, MOD_PROJECTILE, Notice, Outcome, BreakState, dobj_of, dobj_of_mut, hit_log, queue_notice,
};
use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;
use crate::glass_and_props::{T6_MOD_EXPLOSIVE, T6_MOD_IMPACT, T6_MOD_MELEE, T6_MOD_UNKNOWN};
use crate::{ServerModelSource, StepNo};
use glam::Vec3;
use xmodel_runtime::{PoseDonorModel, BreakableChunk};

/// `dflags` bit: the hit is a blast's share, and the pieces were already given theirs by the blast.
const FLAG_BLAST_SHARE: i32 = 1;
/// `dflags` bit: the hit is a script's `dodamage`, taken whole by the entity.
const FLAG_SCRIPT: i32 = 0x10;

/// A script's `dodamage` has this much reach: it hurts like a blast of this radius, `amount` close in and 10 at the edge.
const SCRIPT_BLAST_RADIUS: f32 = 400.0;
const SCRIPT_BLAST_EDGE: f32 = 10.0;

/// A piece whose children BO2's `DestructibleDamage` spares when the prop is a map's.
const SPARING_PIECE: usize = 5;

/// A script's `dodamage` on a Black Ops II map's prop, the way BO2's `ScrCmd_DoDamage` gives it: first a blast from
/// `point` (`DestructibleRadiusDamage`: `amount` close in, 10 at 400 units, by `means`, or `MOD_EXPLOSIVE` for none),
/// then the hit itself (`G_Damage` with `flags` and 0x10, pointed from `point` to the prop, see [`apply_damage`]).
/// False when `owner` is not a destructible.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_script_damage(
    world: &mut StepWorldView,
    tick: StepNo,
    owner: ServerModelSource,
    at: [f32; 3],
    power: f32,
    means: u8,
    kind: DamageKind,
    flags: i32,
    culprit: Option<SeatNo>,
    gun_no: u32,
) -> bool {
    let Some((hosted, prop_origin)) = dobj_of(world, owner).and_then(|dobj| {
        let hosted = dobj.breakable.as_ref()?.hosted;
        Some((hosted, dobj.placement_matrix.w_axis.truncate()))
    }) else {
        return false;
    };
    let mut dir = None;
    if hosted {
        let blast_means = if means == T6_MOD_UNKNOWN { T6_MOD_EXPLOSIVE } else { means };
        let falloff = |d: f32| (1.0 - d / SCRIPT_BLAST_RADIUS) * (power - SCRIPT_BLAST_EDGE) + SCRIPT_BLAST_EDGE;
        let blast = Blast {
            origin: Vec3::from_array(at),
            reach: SCRIPT_BLAST_RADIUS,
            amount_at: &falloff,
            means: blast_means,
            attacker: culprit,
            gun: gun_no,
        };
        radius_pieces(world, tick, owner, &blast);
        dir = (prop_origin - blast.origin).try_normalize().map(|v| v.to_array());
    }
    apply_damage(
        world,
        tick,
        owner,
        None,
        power as i32,
        kind,
        means,
        at,
        dir,
        flags | FLAG_SCRIPT,
        culprit,
        gun_no,
    )
}

/// A bullet (by `means`) striking `bone` of a breakable at `point`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_destructible_hit(
    world: &mut StepWorldView,
    tick: StepNo,
    owner: ServerModelSource,
    bone: u16,
    power: u32,
    means: u8,
    at: [f32; 3],
    dir: [f32; 3],
    culprit: Option<SeatNo>,
    gun: u32,
) -> bool {
    apply_damage(
        world,
        tick,
        owner,
        Some(bone),
        i32::try_from(power).unwrap_or(i32::MAX),
        DamageKind::Bullet,
        means,
        at,
        Some(dir),
        0,
        culprit,
        gun,
    )
}

/// Damages a destructible: the piece whose shown bone (or a bone under it) was struck, else the base piece, by that
/// piece's scale for the kind. A piece it breaks off flies along `dir`. False when `owner` is not a destructible.
///
/// A Black Ops II map's prop takes it the way BO2's `G_Damage` and `DestructibleDamage` give it: at least 1, by `means`,
/// and with no bone, on the first destroyed piece; a blast's share (`flags` & 1) leaves the pieces alone. Then the
/// entity's own health takes what `DestructibleDamage` hands back (a [`Notice::Damage`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_damage(
    world: &mut StepWorldView,
    tick: StepNo,
    owner: ServerModelSource,
    bone: Option<u16>,
    harm: i32,
    kind: DamageKind,
    means: u8,
    at: [f32; 3],
    push: Option<[f32; 3]>,
    flags: i32,
    culprit: Option<SeatNo>,
    gun_no: u32,
) -> bool {
    let burns = burn_rolls(world, owner, means, harm);
    let Some(dobj) = dobj_of_mut(world, owner) else {
        return false;
    };
    let Some(state) = dobj.breakable.as_mut() else {
        return false;
    };
    let hosted = state.hosted;
    let pose = dobj.can_do.as_ref().map(|cap| &cap.pose);
    let index = struck_index(state, pose, bone);
    let (transfer, harm, scale) = {
        let piece = &state.blueprint.pieces[index];
        let (harm, scale) = if hosted {
            (harm.max(1), means_scale(piece, means))
        } else {
            (harm, kind_scale(piece, kind))
        };
        (piece.entity_damage_transfer, harm, scale)
    };
    let damage = (harm as f32 * scale) as i32;

    // BO2's `G_Damage` skips `DestructibleDamage` for a blast's share: `DestructibleRadiusDamage` gave the pieces theirs.
    let pieces = !(hosted && flags & FLAG_BLAST_SHARE != 0);
    if pieces {
        if hit_log() {
            let tag = bone
                .zip(pose)
                .and_then(|(b, pose)| pose.joint_labels.get(usize::from(b)).cloned());
            let shows: Vec<_> = state.blueprint.pieces[index]
                .stages
                .iter()
                .map(|s| s.reveal_joint.clone())
                .collect();
            diag::info!(
                Sim,
                "bo2mp destructible {} hit on {tag:?}: piece {index} of {} (stages show {shows:?}) \
                 takes {damage} ({kind:?} x{scale}), health {:?}",
                state.blueprint.name,
                state.blueprint.pieces.len(),
                state.piece_health
            );
        }
        let out = if burns.is_empty() {
            state.hit_piece(index, damage, means, true, hosted.then_some(SPARING_PIECE))
        } else {
            // Fire is unscaled, and passes none to the pieces under each.
            let mut out = Outcome::default();
            let strike = Strike {
                means,
                down: false,
                spare: None,
            };
            for (piece, burn) in burns.into_iter().enumerate() {
                state.damage(piece, burn, strike, 0, &mut out);
            }
            out
        };
        publish(
            world,
            tick,
            owner,
            out,
            Cause {
                kind,
                dir: push,
                attacker: culprit,
                gun: gun_no,
            },
        );
    }
    if hosted {
        let taken = entity_share(pieces, flags, means, harm, damage, transfer);
        queue_notice(
            world,
            owner,
            Notice::Damage {
                amount: taken,
                attacker: culprit,
                means,
                gun: gun_no,
                at,
                way: push,
                flags,
            },
        );
    }
    true
}

/// BO2's `DestructibleDamage` burns every piece of a map prop by a random 1..=`amount` (`G_rand`): the rolls, one per
/// piece, or none when this hit is no burn on a map prop.
fn burn_rolls(world: &mut StepWorldView, owner: ServerModelSource, means: u8, amount: i32) -> Vec<i32> {
    let pieces = dobj_of(world, owner)
        .and_then(|dobj| dobj.breakable.as_ref())
        .filter(|state| state.hosted && means == MOD_BURNED)
        .map(|state| state.piece_health.len());
    let Some(pieces) = pieces else {
        return Vec::new();
    };
    let dice = world.combat_dice_mut();
    (0..pieces)
        .map(|_| (dice.draw_word() % amount.max(1) as u32) as i32 + 1)
        .collect()
}

/// The piece a hit lands on. A map prop's hit with no bone asks for the piece that shows none, which is the first
/// destroyed one; any other hit with no (or an unknown) bone lands on the base piece.
fn struck_index(state: &BreakState, pose: Option<&PoseDonorModel>, bone: Option<u16>) -> usize {
    if bone.is_none() && state.hosted {
        let destroyed = |i: &usize| state.blueprint.pieces[*i].stage(*i, state.piece_health[*i]).is_none();
        return (0..state.piece_health.len()).find(destroyed).unwrap_or(0);
    }
    bone.zip(pose)
        .and_then(|(bone, pose)| struck_piece(state, pose, usize::from(bone)))
        .unwrap_or(0)
}

/// The piece showing the struck bone, or the nearest bone above it that a piece shows. A Black Ops II map's prop takes
/// the struck bone exactly (BO2's `Destructible_GetPieceIndexFromBoneName`; a bone under it is the base piece's).
fn struck_piece(state: &BreakState, pose: &PoseDonorModel, joint: usize) -> Option<usize> {
    let mut at = joint;
    loop {
        let tag = pose.joint_labels.get(at)?;
        let found = state.blueprint.pieces.iter().enumerate().position(|(i, p)| {
            p.stage(i, state.piece_health[i])
                .is_some_and(|stage| p.stages[stage].reveal_joint.as_ref() == Some(tag))
        });
        if found.is_some() || state.hosted {
            return found;
        }
        let step = usize::from(*pose.ancestor_chain.get(at.checked_sub(pose.top_joint_total)?)?);
        at = at.checked_sub(step).filter(|_| step > 0)?;
    }
}

/// The scale BO2's `DestructibleDamage` counts a hit on `piece` by, from its means of death.
pub(super) fn means_scale(piece: &BreakableChunk, mod_kind: u8) -> f32 {
    match mod_kind {
        MOD_GRENADE | MOD_PROJECTILE | T6_MOD_EXPLOSIVE => piece.blast_damage_multiplier,
        T6_MOD_MELEE => piece.melee_damage_scale,
        T6_MOD_IMPACT => piece.impact_damage_scale,
        _ => piece.round_damage_multiplier,
    }
}

/// The scale a plain T5 prop counts a hit by, from the kind the caller gave.
fn kind_scale(piece: &BreakableChunk, kind: DamageKind) -> f32 {
    match kind {
        DamageKind::Bullet | DamageKind::Splash => piece.round_damage_multiplier,
        DamageKind::Explosive | DamageKind::Blast => piece.blast_damage_multiplier,
        DamageKind::Melee => piece.melee_damage_scale,
        DamageKind::Impact => piece.impact_damage_scale,
        DamageKind::Burned | DamageKind::Script => 1.0,
    }
}

/// What the entity's own health loses: the whole hit for a blast's share, a script's `dodamage` or a burn; nothing for a
/// knife; else the piece's share of what it took (`entityDamageTransfer`).
pub(super) fn entity_share(pieces_hit: bool, flags: i32, means: u8, qty: i32, harm: i32, transfer: f32) -> i32 {
    if !pieces_hit || flags & FLAG_SCRIPT != 0 || means == MOD_BURNED {
        qty
    } else if means == T6_MOD_MELEE || transfer <= 0.0 {
        0
    } else {
        (harm as f32 * transfer) as i32
    }
}
