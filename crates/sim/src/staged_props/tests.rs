use super::hit::{entity_share, means_scale};
use super::launch::launch_velocity;
use super::{DamageKind, MOD_BURNED, MOD_GRENADE, Outcome, BreakState};
use crate::glass_and_props::{T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH, T6_MOD_IMPACT, T6_MOD_MELEE, T6_MOD_RIFLE_BULLET};
use std::sync::Arc;
use xmodel_runtime::{DebrisPreset, BreakableModelDef, BreakableChunk, BreakableStage};

/// A stage that shows `bone` while the piece has at least `shatter_hitpoints` (a share of its health) left.
fn stage(bone: &str, shatter_hitpoints: f32, flags: u32) -> BreakableStage {
    BreakableStage {
        reveal_joint: Some(bone.to_owned()),
        shatter_hitpoints,
        flags,
        ..Default::default()
    }
}

/// A piece of 100 health, every scale 1, made of `stages` (the rest of its five slots empty).
fn piece(stages: Vec<BreakableStage>, parent: u8, share: f32) -> BreakableChunk {
    let mut slots: [BreakableStage; 5] = Default::default();
    for (slot, wanted) in slots.iter_mut().zip(stages) {
        *slot = wanted;
    }
    BreakableChunk {
        stages: slots,
        owner_fragment: parent,
        inherited_harm_ratio: share,
        round_damage_multiplier: 1.0,
        blast_damage_multiplier: 1.0,
        melee_damage_scale: 1.0,
        impact_damage_scale: 1.0,
        entity_damage_transfer: 0.0,
        health: 100,
        conceal_joints: [0; 5],
    }
}

fn prop(pieces: Vec<BreakableChunk>) -> BreakState {
    let local_machine_only = false;
    BreakState {
        piece_health: pieces.iter().map(|p| p.health as i16).collect(),
        blueprint: Arc::new(BreakableModelDef {
            name: "test".to_owned(),
            model: "test".to_owned(),
            pieces,
            local_machine_only,
        }),
        hosted: true,
        notices: Vec::new(),
    }
}

/// A prop of one piece, whole down to half its health and broken below that; `flags` are the whole stage's.
fn single(flags: u32) -> BreakState {
    prop(vec![piece(
        vec![stage("body", 0.5, flags), stage("body_d1", 0.0, 0)],
        255,
        0.0,
    )])
}

fn strike(state: &mut BreakState, slot_at: usize, qty: i32, means: u8, down: bool) -> Outcome {
    state.hit_piece(slot_at, qty, means, down, None)
}

#[test]
fn only_a_stage_that_allows_it_breaks_to_a_knife() {
    let mut closed = single(0);
    strike(&mut closed, 0, 60, T6_MOD_MELEE, true);
    assert_eq!(closed.piece_health, [100]);

    let mut open = single(8);
    strike(&mut open, 0, 60, T6_MOD_MELEE, true);
    assert_eq!(open.piece_health, [40]);
}

#[test]
fn a_holding_stage_gives_way_to_blasts_alone() {
    for means in [T6_MOD_RIFLE_BULLET, MOD_GRENADE, T6_MOD_IMPACT] {
        let mut s = single(1);
        strike(&mut s, 0, 60, means, true);
        assert_eq!(s.piece_health, [100], "means {means}");
    }
    for means in [T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH] {
        let mut s = single(1);
        strike(&mut s, 0, 60, means, true);
        assert_eq!(s.piece_health, [40], "means {means}");
    }
}

#[test]
fn a_hit_that_would_destroy_a_piece_stops_at_the_holding_stage_before_it() {
    // A door: whole, then a stage that holds from a fifth of its health, then the broken stage.
    let mut s = prop(vec![
        piece(vec![stage("body", 0.0, 0)], 255, 0.0),
        piece(
            vec![stage("door", 0.5, 0), stage("door_d1", 0.2, 1), stage("door_d2", 0.0, 0)],
            0,
            0.0,
        ),
    ]);
    strike(&mut s, 1, 500, T6_MOD_RIFLE_BULLET, true);
    assert_eq!(s.piece_health[1], 21);
    strike(&mut s, 1, 500, T6_MOD_RIFLE_BULLET, true);
    assert_eq!(s.piece_health[1], 21);
    strike(&mut s, 1, 500, T6_MOD_EXPLOSIVE, true);
    assert_eq!(s.piece_health[1], -1);
}

#[test]
fn fire_does_not_pass_down_to_the_pieces_under_the_one_it_burns() {
    let car = || {
        prop(vec![
            piece(vec![stage("body", 0.0, 0)], 255, 0.0),
            piece(vec![stage("hood", 0.0, 0)], 0, 0.5),
        ])
    };
    let mut burned = car();
    strike(&mut burned, 0, 40, MOD_BURNED, false);
    assert_eq!(burned.piece_health, [60, 100]);

    let mut shot = car();
    strike(&mut shot, 0, 40, T6_MOD_RIFLE_BULLET, true);
    assert_eq!(shot.piece_health, [60, 80]);
}

#[test]
fn a_spared_piece_keeps_its_children_whole() {
    let mut s = prop(vec![
        piece(vec![stage("body", 0.0, 0)], 255, 0.0),
        piece(vec![stage("hood", 0.0, 0)], 0, 0.5),
    ]);
    s.hit_piece(0, 40, T6_MOD_RIFLE_BULLET, true, Some(0));
    assert_eq!(s.piece_health, [60, 100]);
}

#[test]
fn the_base_piece_plays_the_loop_of_the_stage_it_enters() {
    let mut s = single(0);
    Arc::make_mut(&mut s.blueprint).pieces[0].stages[1].repeating_audio = Some("fire".to_owned());
    let first = strike(&mut s, 0, 60, T6_MOD_RIFLE_BULLET, true);
    assert_eq!(first.base_loop, Some(Some("fire".to_owned())));
    assert!(!first.died);

    // Destroyed, it keeps its last stage's look: no loop of its own any more, and it has died.
    let last = strike(&mut s, 0, 100, T6_MOD_RIFLE_BULLET, true);
    assert_eq!(last.base_loop, None);
    assert!(last.died);
    assert_eq!(s.piece_health, [-1]);
}

#[test]
fn each_means_of_death_has_its_own_scale() {
    let mut p = piece(vec![stage("body", 0.0, 0)], 255, 0.0);
    p.round_damage_multiplier = 0.1;
    p.blast_damage_multiplier = 0.2;
    p.melee_damage_scale = 0.3;
    p.impact_damage_scale = 0.4;
    assert_eq!(means_scale(&p, T6_MOD_RIFLE_BULLET), 0.1);
    assert_eq!(means_scale(&p, MOD_GRENADE), 0.2);
    assert_eq!(means_scale(&p, T6_MOD_EXPLOSIVE), 0.2);
    assert_eq!(means_scale(&p, T6_MOD_MELEE), 0.3);
    assert_eq!(means_scale(&p, T6_MOD_IMPACT), 0.4);
}

#[test]
fn the_entity_loses_its_share_of_a_hit() {
    // The whole hit when it skipped the pieces, came from a script or burned.
    assert_eq!(entity_share(false, 0, T6_MOD_RIFLE_BULLET, 50, 25, 0.5), 50);
    assert_eq!(entity_share(true, 0x10, T6_MOD_RIFLE_BULLET, 50, 25, 0.5), 50);
    assert_eq!(entity_share(true, 0, MOD_BURNED, 50, 25, 0.5), 50);
    // None for a knife or a piece that transfers nothing; else the transferred part of what the piece took.
    assert_eq!(entity_share(true, 0, T6_MOD_MELEE, 50, 25, 0.5), 0);
    assert_eq!(entity_share(true, 0, T6_MOD_RIFLE_BULLET, 50, 25, 0.0), 0);
    assert_eq!(entity_share(true, 0, T6_MOD_RIFLE_BULLET, 50, 25, 0.5), 12);
}

fn preset(mass: f32) -> DebrisPreset {
    DebrisPreset {
        mass,
        bounce: 0.0,
        friction: 0.0,
        bullet_push: 1.0,
        blast_push: 1.0,
        gravity_scale: 1.0,
    }
}

#[test]
fn a_piece_with_nothing_to_push_it_just_drops() {
    assert_eq!(launch_velocity(&preset(1.0), DamageKind::Bullet, None), [0.0; 3]);
    assert_eq!(launch_velocity(&preset(1.0), DamageKind::Bullet, Some([0.0; 3])), [0.0; 3]);
    assert_eq!(launch_velocity(&preset(1.0), DamageKind::Script, Some([1.0, 0.0, 0.0])), [0.0; 3]);
}

#[test]
fn a_bullet_pushes_along_its_way_and_a_heavier_piece_moves_less() {
    let light = launch_velocity(&preset(1.0), DamageKind::Bullet, Some([2.0, 0.0, 0.0]));
    assert_eq!(light, [0.8, 0.0, 0.0]);
    let heavy = launch_velocity(&preset(4.0), DamageKind::Bullet, Some([2.0, 0.0, 0.0]));
    assert!((heavy[0] - 0.2).abs() < 1e-6 && heavy[1] == 0.0 && heavy[2] == 0.0);
}

#[test]
fn a_blast_lifts_what_it_throws_and_a_weak_one_throws_nothing() {
    let thrown = launch_velocity(&preset(1.0), DamageKind::Blast, Some([1.0, 0.0, 0.0]));
    assert!(thrown[0] > 0.0 && thrown[2] > 0.0, "{thrown:?}");
    // A blast's direction shrinks with the falloff; far out, the push falls under the minimum.
    let weak = launch_velocity(&preset(1.0), DamageKind::Blast, Some([0.0005, 0.0, 0.0]));
    assert_eq!(weak, [0.0; 3]);
}
