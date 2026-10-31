//! What gunfire, knife swipes and blasts do to loose pieces on the map: knock
//! them about, wound the breakable ones, and when one gives out hide it, free
//! its physics body and play the effect its definition names.

use assets::StagedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use entity_t6::event;
use net::{BulletStrike, SoundCueAt, BlastAt};
use render_fx::{EntityMarkBoard, EffectsSystemHandle, ReadyFxBook};

use super::force::{
    blast_damage, blast_radii, blast_reach, blast_shove, segment_graze, strike_shove, take_damage,
};
use super::grid::WakeGrid;
use crate::adapters::motion::free_body_wake::{is_breakable, wakeable};
use crate::adapters::motion::debris_piece::PieceModels;
use crate::adapters::motion::debris_phys::{PropPhysicsNudge, DebrisPhysLevel};
use render_fx::effect_play::{EffectElementInfoMemo, play_named_oriented_in_level};
use crate::adapters::fx::world_decal::FrontendEffectStage;
use crate::prepare::scene::cull::DynamicPropModelEntity;
use crate::prepare::scene::world::{LevelDebrisInstance, LevelStage};

type PieceQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut LevelDebrisInstance,
        &'static Transform,
        &'static mut Visibility,
    ),
    With<DynamicPropModelEntity>,
>;

/// Everything the hit observers need to find pieces and act on them.
#[derive(SystemParam)]
pub(super) struct Rubble<'w, 's> {
    grid: ResMut<'w, WakeGrid>,
    pieces: PieceQuery<'w, 's>,
    pushes: MessageWriter<'w, PropPhysicsNudge>,
    bodies: ResMut<'w, DebrisPhysLevel>,
    effect_host: Option<ResMut<'w, EffectsSystemHandle>>,
    effect_ledger: Option<Res<'w, ReadyFxBook>>,
    fx_scene: Option<Res<'w, LevelStage>>,
    mark_board: Res<'w, EntityMarkBoard>,
    gun_table: Option<Res<'w, StagedWeapons>>,
    map_models: Option<Res<'w, asset_world::LevelModelPlacementRoster>>,
    t6_models: Option<Res<'w, assets::ReadiedFpvMeshes>>,
}

/// One blast: where it went off, how far it reaches at full and at no
/// strength, what it does to breakable pieces there, and the heading its
/// event carried (zero to push straight away from the centre).
struct Blast {
    pos: Vec3,
    authored: Vec3,
    inner: f32,
    outer: f32,
    core_damage: i32,
    edge_damage: i32,
}

/// The first wakeable piece a path runs into.
struct PathHit {
    ent: Entity,
    mid: Vec3,
    push_scale: f32,
}

impl Rubble<'_, '_> {
    /// The blast `weapon` makes at `origin`, if it makes one.
    fn blast_of(&self, gun_no: u32, at_pos: Vec3, authored: Vec3) -> Option<Blast> {
        let facts = self.gun_table.as_ref()?.0.summary_for(gun_no)?;
        let (inner, outer) = blast_radii(facts.blast_outer_reach, facts.blast_reach_floor)?;
        Some(Blast {
            pos: at_pos,
            authored,
            inner,
            outer,
            core_damage: facts.blast_core_damage,
            edge_damage: facts.blast_rim_damage,
        })
    }

    /// What a shot of `weapon` takes off a breakable piece.
    fn hit_damage(&self, weapon_id: u32) -> i32 {
        let facts = self.gun_table.as_ref().and_then(|w| w.0.summary_for(weapon_id));
        facts.map_or(0, |facts| facts.damage)
    }

    /// Shoves and wounds every piece the blast reaches.
    fn detonate(&mut self, blast: Blast) {
        let near = self.grid.near_sphere(
            blast.pos,
            blast.outer,
            self.pieces.iter().map(|piece| piece.0),
        );
        let mut breaker = Breaker {
            bodies: &mut self.bodies,
            host: self.effect_host.as_deref_mut(),
            catalog: self.effect_ledger.as_deref(),
            scene: self.fx_scene.as_deref(),
            marks: &self.mark_board,
        };
        for &ent in near {
            let Ok((_, mut piece, at, mut shown)) = self.pieces.get_mut(ent) else {
                continue;
            };
            if piece.gone {
                continue;
            }
            let distance = at.translation.distance(blast.pos);
            let Some(reach) = blast_reach(distance, blast.inner, blast.outer) else {
                continue;
            };
            if let Some(wake_preset) = wakeable(&piece) {
                let shove = blast_shove(
                    at.translation - blast.pos,
                    reach,
                    wake_preset.blast_push_gain,
                    blast.authored,
                );
                if let Some(impulse) = shove {
                    self.pushes.write(PropPhysicsNudge { entity: ent, impulse });
                }
            }
            let damage = blast_damage(blast.core_damage, blast.edge_damage, reach);
            breaker.hurt(ent, &mut piece, at, &mut shown, damage);
        }
    }

    /// A bullet or knife swipe from `from` to `to`: shoves the first piece
    /// on its path and wounds it for `damage`.
    fn strike(&mut self, from: Vec3, to: Vec3, damage: i32) {
        let Some(hit) = self.first_on_path(from, to) else {
            return;
        };
        let path = to - from;
        let heading = if path.length_squared() < 1e-12 {
            hit.mid - from
        } else {
            path
        };
        if let Some(impulse) = strike_shove(heading, hit.push_scale) {
            self.pushes.write(PropPhysicsNudge {
                entity: hit.ent,
                impulse,
            });
        }
        let Ok((_, mut piece, at, mut shown)) = self.pieces.get_mut(hit.ent) else {
            return;
        };
        let mut breaker = Breaker {
            bodies: &mut self.bodies,
            host: self.effect_host.as_deref_mut(),
            catalog: self.effect_ledger.as_deref(),
            scene: self.fx_scene.as_deref(),
            marks: &self.mark_board,
        };
        breaker.hurt(hit.ent, &mut piece, at, &mut shown, damage);
    }

    /// Of the wakeable pieces whose bounding sphere the path passes through,
    /// the one it reaches first (the earlier one wins a tie).
    fn first_on_path(&mut self, from: Vec3, to: Vec3) -> Option<PathHit> {
        let models = PieceModels {
            map: self.map_models.as_deref(),
            t6: self.t6_models.as_deref(),
        };
        let near = self
            .grid
            .near_segment(from, to, self.pieces.iter().map(|piece| piece.0));
        let mut nearest: Option<(f32, PathHit)> = None;
        for &entity in near {
            let Ok((_, piece, at, _)) = self.pieces.get(entity) else {
                continue;
            };
            let Some(preset) = wakeable(piece) else {
                continue;
            };
            let reach = models.radius(&piece.active_mesh).max(1.0);
            let Some(along) = segment_graze(from, to, at.translation, reach) else {
                continue;
            };
            if nearest.as_ref().is_none_or(|(so_far, _)| along < *so_far) {
                let hit = PathHit {
                    ent: entity,
                    mid: at.translation,
                    push_scale: preset.shot_push_gain,
                };
                nearest = Some((along, hit));
            }
        }
        nearest.map(|(_, hit)| hit)
    }
}

/// What it takes to break a piece, borrowed for the length of one event.
struct Breaker<'a> {
    bodies: &'a mut DebrisPhysLevel,
    host: Option<&'a mut EffectsSystemHandle>,
    catalog: Option<&'a ReadyFxBook>,
    scene: Option<&'a LevelStage>,
    marks: &'a EntityMarkBoard,
}

impl Breaker<'_> {
    /// Wounds a breakable piece. The hit that finishes it hides it, frees its
    /// physics body and plays its break effect; other pieces ignore damage.
    fn hurt(
        &mut self,
        ent: Entity,
        piece: &mut LevelDebrisInstance,
        at: &Transform,
        shown: &mut Visibility,
        dmg: i32,
    ) {
        if !is_breakable(piece.ty) || !take_damage(&mut piece.hp, dmg) {
            return;
        }
        piece.gone = true;
        *shown = Visibility::Hidden;
        self.bodies.remove_physics_body(ent);
        if let Some(name) = piece.destroy_effect.as_deref() {
            self.play_break_effect(name, at);
        }
    }

    /// Plays the named effect where the piece stood, turned the way it was.
    /// Without an effects host or catalog there is nothing to play it with.
    fn play_break_effect(&mut self, name: &str, at: &Transform) {
        let (Some(fx_host), Some(fx_catalog)) = (self.host.as_deref_mut(), self.catalog) else {
            return;
        };
        let scene = FrontendEffectStage::wrap(self.scene, self.marks);
        let _ = play_named_oriented_in_level(
            &mut fx_host.0,
            &fx_catalog.0,
            &EffectElementInfoMemo::default(),
            fx_catalog.0.map_fx_name(name),
            at.translation.to_array(),
            basis_axes(at.rotation),
            scene
                .as_ref()
                .map(|s| s as &dyn render_fx::effect_play::EffectStage),
        );
    }
}

/// The three axes of a rotation as the effects code wants them.
fn basis_axes(rotation: Quat) -> [[f32; 3]; 3] {
    let basis = Mat3::from_quat(rotation);
    [
        basis.x_axis.to_array(),
        basis.y_axis.to_array(),
        basis.z_axis.to_array(),
    ]
}

/// Blasts that push pieces straight away from their centre rather than along
/// a heading of their own.
fn is_radial_blast(event_id: i32) -> bool {
    matches!(
        u8::try_from(event_id),
        Ok(event::BOMB_BURST
            | event::ROCKET_EXPLODE
            | event::ROCKET_EXPLODE_NOMARKS
            | event::FLASHBANG_EXPLODE)
    )
}

pub(super) fn on_blast(blast: On<BlastAt>, mut rubble: Rubble) {
    let event = &blast.event;
    // A radial blast pushes outward from its centre; others keep their aim.
    let push = (!is_radial_blast(event.event.0)).then(|| event.payload.direction.into());
    let centre: Vec3 = event.payload.origin.into();
    if let Some(found) = rubble.blast_of(event.payload.weapon, centre, push.unwrap_or(Vec3::ZERO)) {
        rubble.detonate(found);
    }
}

pub(super) fn on_bullet(hit: On<BulletStrike>, mut rubble: Rubble) {
    let shot = hit.event.payload;
    let damage = rubble.hit_damage(shot.weapon);
    rubble.strike(
        Vec3::from_array(shot.origin2),
        Vec3::from_array(shot.origin),
        damage,
    );
}

/// A knife hit arrives as a sound event; nothing else in that stream moves
/// pieces.
pub(super) fn on_knife(sound: On<SoundCueAt>, mut rubble: Rubble) {
    if sound.event.event.0 != i32::from(event::MELEE_HIT) {
        return;
    }
    let swing = sound.event.payload;
    let damage = rubble.hit_damage(swing.weapon);
    rubble.strike(
        Vec3::from_array(swing.origin2),
        Vec3::from_array(swing.origin),
        damage,
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use asset_world::MovableObjectKind;
    use bevy::ecs::system::SystemState;

    use super::super::force::{DEBRIS_BULLET_FORCE, DEBRIS_EXPLODE_FORCE};
    use super::super::sample::piece;
    use super::*;

    fn break_with(piece: &mut LevelDebrisInstance, damage: i32) -> (bool, Visibility) {
        let mut bodies = DebrisPhysLevel::default();
        let marks = EntityMarkBoard::default();
        let mut breaker = Breaker {
            bodies: &mut bodies,
            host: None,
            catalog: None,
            scene: None,
            marks: &marks,
        };
        let mut shown = Visibility::Inherited;
        breaker.hurt(
            Entity::PLACEHOLDER,
            piece,
            &Transform::default(),
            &mut shown,
            damage,
        );
        (piece.gone, shown)
    }

    #[test]
    fn the_finishing_hit_breaks_and_hides_the_piece() {
        let mut p = piece(MovableObjectKind::Destruct, 1.0);
        p.hp = 30;
        p.destroy_effect = Some("props/glass_break".into());
        let (dead, shown) = break_with(&mut p, 45);
        assert!(dead);
        assert_eq!(p.hp, 0);
        assert_eq!(shown, Visibility::Hidden);
    }

    #[test]
    fn a_wound_that_does_not_finish_it_leaves_it_standing() {
        let mut p = LevelDebrisInstance { hp: 100, ..piece(MovableObjectKind::Destruct, 1.0) };
        let (dead, shown) = break_with(&mut p, 40);
        assert!(!dead);
        assert_eq!(p.hp, 60);
        assert_eq!(shown, Visibility::Inherited);
    }

    #[test]
    fn clutter_cannot_be_broken() {
        let mut p = piece(MovableObjectKind::Clutter, 1.0);
        p.hp = 10;
        let (dead, shown) = break_with(&mut p, 500);
        assert!(!dead);
        assert_eq!(p.hp, 10);
        assert_eq!(shown, Visibility::Inherited);
    }

    #[test]
    fn a_broken_piece_stays_broken_quietly() {
        let mut p = piece(MovableObjectKind::Destruct, 1.0);
        p.hp = 0;
        p.gone = true;
        let (dead, shown) = break_with(&mut p, 500);
        assert!(dead);
        assert_eq!(shown, Visibility::Inherited);
    }

    #[test]
    fn the_basis_follows_the_rotation() {
        let identity = basis_axes(Quat::IDENTITY);
        assert_eq!(identity, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        let quarter = basis_axes(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!((Vec3::from_array(quarter[0]) - Vec3::Y).length() < 1e-6);
        assert!((Vec3::from_array(quarter[1]) + Vec3::X).length() < 1e-6);
        assert_eq!(quarter[2], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn only_the_four_plain_blasts_ignore_the_authored_heading() {
        for id in [
            event::BOMB_BURST,
            event::ROCKET_EXPLODE,
            event::ROCKET_EXPLODE_NOMARKS,
            event::FLASHBANG_EXPLODE,
        ] {
            assert!(is_radial_blast(i32::from(id)));
        }
        assert!(!is_radial_blast(i32::from(event::MELEE_HIT)));
        assert!(!is_radial_blast(-1));
        assert!(!is_radial_blast(100_000));
    }

    /// A world with just what `Rubble` asks for (no weapons, models or
    /// effects), and a way to add pieces to it.
    struct Yard {
        world: World,
    }

    impl Yard {
        fn new() -> Self {
            let mut world = World::new();
            world.init_resource::<WakeGrid>();
            world.init_resource::<DebrisPhysLevel>();
            world.init_resource::<EntityMarkBoard>();
            world.init_resource::<Messages<PropPhysicsNudge>>();
            Self { world }
        }

        /// Adds a piece and refiles every piece (each with a reach of 1, as a
        /// piece with no known model gets).
        fn insert_at(&mut self, piece: LevelDebrisInstance, at: Vec3) -> Entity {
            let ident = self
                .world
                .spawn((
                    piece,
                    Transform::from_translation(at),
                    Visibility::Inherited,
                    DynamicPropModelEntity,
                ))
                .id();
            let all: Vec<_> = self
                .world
                .query_filtered::<(&Transform, Entity), With<DynamicPropModelEntity>>()
                .iter(&self.world)
                .map(|(at, ent)| (ent, at.translation, 1.0))
                .collect();
            self.world.resource_mut::<WakeGrid>().refill(all);
            ident
        }

        fn with_rubble<R>(&mut self, act: impl FnOnce(&mut Rubble) -> R) -> R {
            let mut state = SystemState::<Rubble>::new(&mut self.world);
            let mut rubble = state.get_mut(&mut self.world).unwrap();
            act(&mut rubble)
        }

        /// The shoves handed out since the last call, by piece.
        fn shoves(&mut self) -> HashMap<Entity, Vec3> {
            let mut sent = self.world.resource_mut::<Messages<PropPhysicsNudge>>();
            sent.drain().map(|m| (m.entity, m.impulse)).collect()
        }

        fn health(&self, piece: Entity) -> i32 {
            self.world.get::<LevelDebrisInstance>(piece).unwrap().hp
        }

        fn is_hidden(&self, piece: Entity) -> bool {
            self.world.get::<Visibility>(piece) == Some(&Visibility::Hidden)
        }
    }

    #[test]
    fn a_shot_shoves_the_first_piece_on_its_path() {
        let mut yard = Yard::new();
        let near = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(50.0, 0.0, 0.0));
        let far = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(120.0, 0.0, 0.0));
        let off_path = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(80.0, 40.0, 0.0));
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(200.0, 0.0, 0.0), 0));
        let shoves = yard.shoves();
        assert_eq!(shoves.len(), 1);
        assert_eq!(shoves[&near], Vec3::new(DEBRIS_BULLET_FORCE, 0.0, 0.0));
        assert!(!shoves.contains_key(&far) && !shoves.contains_key(&off_path));
    }

    #[test]
    fn a_shot_skips_pieces_that_cannot_be_woken() {
        let mut yard = Yard::new();
        let mut broken = piece(MovableObjectKind::Clutter, 1.0);
        broken.gone = true;
        yard.insert_at(broken, Vec3::new(20.0, 0.0, 0.0));
        yard.insert_at(piece(MovableObjectKind::Invalid, 1.0), Vec3::new(30.0, 0.0, 0.0));
        let behind = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(90.0, 0.0, 0.0));
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(200.0, 0.0, 0.0), 0));
        let shoves = yard.shoves();
        assert_eq!(shoves.keys().copied().collect::<Vec<_>>(), vec![behind]);
    }

    #[test]
    fn a_shot_that_finishes_a_breakable_piece_breaks_it() {
        let mut yard = Yard::new();
        let mut glass = piece(MovableObjectKind::Destruct, 1.0);
        glass.hp = 40;
        let glass = yard.insert_at(glass, Vec3::new(50.0, 0.0, 0.0));
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), 25));
        assert_eq!(yard.health(glass), 15);
        assert!(!yard.is_hidden(glass));
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), 25));
        assert_eq!(yard.health(glass), 0);
        assert!(yard.is_hidden(glass));
        // Broken glass is out of the way: the next shot finds nothing.
        yard.shoves();
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), 25));
        assert!(yard.shoves().is_empty());
    }

    #[test]
    fn a_shot_with_no_length_is_aimed_at_the_piece() {
        let mut yard = Yard::new();
        let crate_ = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::Y * 3.0);
        let from = Vec3::new(0.0, 2.5, 0.0);
        yard.with_rubble(|r| r.strike(from, from, 0));
        let shoves = yard.shoves();
        assert_eq!(shoves[&crate_], Vec3::Y * DEBRIS_BULLET_FORCE);
    }

    #[test]
    fn a_blast_shoves_and_wounds_by_distance() {
        let mut yard = Yard::new();
        let crate_ = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(50.0, 0.0, 0.0));
        let glass = LevelDebrisInstance { hp: 100, ..piece(MovableObjectKind::Destruct, 1.0) };
        let glass = yard.insert_at(glass, Vec3::Y * 200.0);
        let vase = LevelDebrisInstance { hp: 100, ..piece(MovableObjectKind::Destruct, 1.0) };
        let vase = yard.insert_at(vase, Vec3::Z * 50.0);
        let outside = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(400.0, 0.0, 0.0));
        yard.with_rubble(|r| {
            r.detonate(Blast {
                pos: Vec3::ZERO,
                authored: Vec3::ZERO,
                inner: 100.0,
                outer: 300.0,
                core_damage: 120,
                edge_damage: 20,
            })
        });

        let shoves = yard.shoves();
        assert_eq!(shoves.len(), 3);
        assert!(!shoves.contains_key(&outside));
        // Full strength straight along x, tipped up by the lift.
        let tilt = Vec3::new(1.0, 0.0, 0.5).normalize();
        assert!((shoves[&crate_] - tilt * DEBRIS_EXPLODE_FORCE).length() < 0.5);
        // Halfway out the strength halves.
        let tilt = Vec3::new(0.0, 1.0, 0.5).normalize();
        assert!((shoves[&glass] - tilt * DEBRIS_EXPLODE_FORCE * 0.5).length() < 0.5);
        // Straight up already points upward, so the lift only adds to it.
        let up = Vec3::new(0.0, 0.0, 1.5).normalize();
        assert!((shoves[&vase] - up * DEBRIS_EXPLODE_FORCE).length() < 0.5);

        // 20 + 100 * 0.5 = 70 off the glass; the 120 at the centre finishes the vase.
        assert_eq!(yard.health(glass), 30);
        assert!(!yard.is_hidden(glass));
        assert_eq!(yard.health(vase), 0);
        assert!(yard.is_hidden(vase));
    }

    #[test]
    fn a_blast_with_its_own_heading_pushes_that_way() {
        let mut yard = Yard::new();
        let crate_ = yard.insert_at(piece(MovableObjectKind::Clutter, 1.0), Vec3::new(50.0, 0.0, 0.0));
        yard.with_rubble(|r| {
            r.detonate(Blast {
                pos: Vec3::ZERO,
                authored: Vec3::new(0.0, 0.0, 1.0),
                inner: 100.0,
                outer: 300.0,
                core_damage: 0,
                edge_damage: 0,
            })
        });
        let shoves = yard.shoves();
        assert!((shoves[&crate_] - Vec3::new(0.0, 0.0, DEBRIS_EXPLODE_FORCE)).length() < 0.5);
    }

    #[test]
    fn without_weapon_tables_nothing_goes_off() {
        let mut yard = Yard::new();
        yard.insert_at(piece(MovableObjectKind::Destruct, 1.0), Vec3::new(10.0, 0.0, 0.0));
        assert!(yard.with_rubble(|r| r.blast_of(7, Vec3::splat(0.0), Vec3::splat(0.0)).is_none()));
        assert_eq!(yard.with_rubble(|r| r.hit_damage(7)), 0);
        assert!(yard.shoves().is_empty());
    }
}
