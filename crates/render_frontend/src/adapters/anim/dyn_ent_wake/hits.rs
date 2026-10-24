//! What gunfire, knife swipes and blasts do to loose pieces on the map: knock
//! them about, wound the breakable ones, and when one gives out hide it, free
//! its physics body and play the effect its definition names.

use assets::PreparedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use entity_t6::event as ev;
use net::{EntityBulletHit, EntityEventSound, EntityExplosion};
use render_fx::{EntityMarks, HostFxSystem, PreparedFxCatalog};

use super::force::{
    blast_damage, blast_radii, blast_reach, blast_shove, segment_graze, strike_shove, take_damage,
};
use super::grid::WakeGrid;
use super::{is_breakable, wakeable};
use crate::adapters::anim::dyn_ent::PieceModels;
use crate::adapters::anim::dyn_ent_phys::{DynEntPhysImpulse, DynEntPhysWorld};
use crate::adapters::fx::present::{FxElemInfoCache, play_named_oriented_in_world};
use crate::adapters::fx::world_mark::FrontendFxScene;
use crate::prepare::scene::cull::DynEntModelEntity;
use crate::prepare::scene::world::{WorldDynEntInstance, WorldScene};

type PieceQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut WorldDynEntInstance,
        &'static Transform,
        &'static mut Visibility,
    ),
    With<DynEntModelEntity>,
>;

/// Everything the hit observers need to find pieces and act on them.
#[derive(SystemParam)]
pub(super) struct Rubble<'w, 's> {
    grid: ResMut<'w, WakeGrid>,
    pieces: PieceQuery<'w, 's>,
    pushes: MessageWriter<'w, DynEntPhysImpulse>,
    bodies: ResMut<'w, DynEntPhysWorld>,
    fx_host: Option<ResMut<'w, HostFxSystem>>,
    fx_catalog: Option<Res<'w, PreparedFxCatalog>>,
    fx_scene: Option<Res<'w, WorldScene>>,
    marks: Res<'w, EntityMarks>,
    weapons: Option<Res<'w, PreparedWeapons>>,
    map_models: Option<Res<'w, asset_world::MapXModelSceneCatalog>>,
    t6_models: Option<Res<'w, assets::PreparedFpvMeshes>>,
}

/// One blast: where it went off, how far it reaches at full and at no
/// strength, what it does to breakable pieces there, and the heading its
/// event carried (zero to push straight away from the centre).
struct Blast {
    origin: Vec3,
    authored: Vec3,
    inner: f32,
    outer: f32,
    inner_damage: i32,
    outer_damage: i32,
}

/// The first wakeable piece a path runs into.
struct PathHit {
    entity: Entity,
    center: Vec3,
    push_scale: f32,
}

impl Rubble<'_, '_> {
    /// The blast `weapon` makes at `origin`, if it makes one.
    fn blast_of(&self, weapon: u32, origin: Vec3, authored: Vec3) -> Option<Blast> {
        let facts = self.weapons.as_ref()?.0.facts_of(weapon)?;
        let (inner, outer) = blast_radii(facts.explosion_radius, facts.explosion_radius_min)?;
        Some(Blast {
            origin,
            authored,
            inner,
            outer,
            inner_damage: facts.explosion_inner_damage,
            outer_damage: facts.explosion_outer_damage,
        })
    }

    /// What a shot of `weapon` takes off a breakable piece.
    fn hit_damage(&self, weapon: u32) -> i32 {
        let facts = self.weapons.as_ref().and_then(|w| w.0.facts_of(weapon));
        facts.map_or(0, |facts| facts.damage)
    }

    /// Shoves and wounds every piece the blast reaches.
    fn detonate(&mut self, blast: Blast) {
        let near = self.grid.near_sphere(
            blast.origin,
            blast.outer,
            self.pieces.iter().map(|piece| piece.0),
        );
        let mut breaker = Breaker {
            bodies: &mut self.bodies,
            host: self.fx_host.as_deref_mut(),
            catalog: self.fx_catalog.as_deref(),
            scene: self.fx_scene.as_deref(),
            marks: &self.marks,
        };
        for &entity in near {
            let Ok((_, mut piece, at, mut shown)) = self.pieces.get_mut(entity) else {
                continue;
            };
            if piece.dead {
                continue;
            }
            let distance = at.translation.distance(blast.origin);
            let Some(reach) = blast_reach(distance, blast.inner, blast.outer) else {
                continue;
            };
            if let Some(preset) = wakeable(&piece) {
                let shove = blast_shove(
                    at.translation - blast.origin,
                    reach,
                    preset.explosive_force_scale,
                    blast.authored,
                );
                if let Some(impulse) = shove {
                    self.pushes.write(DynEntPhysImpulse { entity, impulse });
                }
            }
            let damage = blast_damage(blast.inner_damage, blast.outer_damage, reach);
            breaker.hurt(entity, &mut piece, at, &mut shown, damage);
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
            hit.center - from
        } else {
            path
        };
        if let Some(impulse) = strike_shove(heading, hit.push_scale) {
            self.pushes.write(DynEntPhysImpulse {
                entity: hit.entity,
                impulse,
            });
        }
        let Ok((_, mut piece, at, mut shown)) = self.pieces.get_mut(hit.entity) else {
            return;
        };
        let mut breaker = Breaker {
            bodies: &mut self.bodies,
            host: self.fx_host.as_deref_mut(),
            catalog: self.fx_catalog.as_deref(),
            scene: self.fx_scene.as_deref(),
            marks: &self.marks,
        };
        breaker.hurt(hit.entity, &mut piece, at, &mut shown, damage);
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
        let mut best: Option<(f32, PathHit)> = None;
        for &entity in near {
            let Ok((_, piece, at, _)) = self.pieces.get(entity) else {
                continue;
            };
            let Some(preset) = wakeable(piece) else {
                continue;
            };
            let reach = models.radius(&piece.current_model).max(1.0);
            let Some(along) = segment_graze(from, to, at.translation, reach) else {
                continue;
            };
            if best.as_ref().is_none_or(|(so_far, _)| along < *so_far) {
                let hit = PathHit {
                    entity,
                    center: at.translation,
                    push_scale: preset.bullet_force_scale,
                };
                best = Some((along, hit));
            }
        }
        best.map(|(_, hit)| hit)
    }
}

/// What it takes to break a piece, borrowed for the length of one event.
struct Breaker<'a> {
    bodies: &'a mut DynEntPhysWorld,
    host: Option<&'a mut HostFxSystem>,
    catalog: Option<&'a PreparedFxCatalog>,
    scene: Option<&'a WorldScene>,
    marks: &'a EntityMarks,
}

impl Breaker<'_> {
    /// Wounds a breakable piece. The hit that finishes it hides it, frees its
    /// physics body and plays its break effect; other pieces ignore damage.
    fn hurt(
        &mut self,
        entity: Entity,
        piece: &mut WorldDynEntInstance,
        at: &Transform,
        shown: &mut Visibility,
        damage: i32,
    ) {
        if !is_breakable(piece.ty) || !take_damage(&mut piece.health, damage) {
            return;
        }
        piece.dead = true;
        *shown = Visibility::Hidden;
        self.bodies.destroy_body(entity);
        if let Some(name) = piece.destroy_fx.as_deref() {
            self.play_break_effect(name, at);
        }
    }

    /// Plays the named effect where the piece stood, turned the way it was.
    /// Without an effects host or catalog there is nothing to play it with.
    fn play_break_effect(&mut self, name: &str, at: &Transform) {
        let (Some(host), Some(catalog)) = (self.host.as_deref_mut(), self.catalog) else {
            return;
        };
        let scene = FrontendFxScene::wrap(self.scene, self.marks);
        let _ = play_named_oriented_in_world(
            &mut host.0,
            &catalog.0,
            &FxElemInfoCache::default(),
            catalog.0.map_fx_name(name),
            at.translation.to_array(),
            basis_axes(at.rotation),
            scene
                .as_ref()
                .map(|s| s as &dyn render_fx::present::FxScene),
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
        Ok(ev::GRENADE_EXPLODE
            | ev::ROCKET_EXPLODE
            | ev::ROCKET_EXPLODE_NOMARKS
            | ev::FLASHBANG_EXPLODE)
    )
}

pub(super) fn on_blast(blast: On<EntityExplosion>, mut rubble: Rubble) {
    let payload = blast.event.payload;
    let authored = if is_radial_blast(blast.event.event.0) {
        Vec3::ZERO
    } else {
        Vec3::from_array(payload.direction)
    };
    let origin = Vec3::from_array(payload.origin);
    if let Some(blast) = rubble.blast_of(payload.weapon, origin, authored) {
        rubble.detonate(blast);
    }
}

pub(super) fn on_bullet(hit: On<EntityBulletHit>, mut rubble: Rubble) {
    let payload = hit.event.payload;
    let damage = rubble.hit_damage(payload.weapon);
    rubble.strike(
        Vec3::from_array(payload.origin2),
        Vec3::from_array(payload.origin),
        damage,
    );
}

/// A knife hit arrives as a sound event; nothing else in that stream moves
/// pieces.
pub(super) fn on_knife(sound: On<EntityEventSound>, mut rubble: Rubble) {
    if sound.event.event.0 != i32::from(ev::MELEE_HIT) {
        return;
    }
    let payload = sound.event.payload;
    let damage = rubble.hit_damage(payload.weapon);
    rubble.strike(
        Vec3::from_array(payload.origin2),
        Vec3::from_array(payload.origin),
        damage,
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use asset_world::DynEntType;
    use bevy::ecs::system::SystemState;

    use super::super::force::{DYNENT_BULLET_FORCE, DYNENT_EXPLODE_FORCE};
    use super::super::sample::piece;
    use super::*;

    fn break_with(piece: &mut WorldDynEntInstance, damage: i32) -> (bool, Visibility) {
        let mut bodies = DynEntPhysWorld::default();
        let marks = EntityMarks::default();
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
        (piece.dead, shown)
    }

    #[test]
    fn the_finishing_hit_breaks_and_hides_the_piece() {
        let mut p = piece(DynEntType::Destruct, 1.0);
        p.health = 30;
        p.destroy_fx = Some("props/glass_break".into());
        let (dead, shown) = break_with(&mut p, 45);
        assert!(dead);
        assert_eq!(p.health, 0);
        assert_eq!(shown, Visibility::Hidden);
    }

    #[test]
    fn a_wound_that_does_not_finish_it_leaves_it_standing() {
        let mut p = piece(DynEntType::Destruct, 1.0);
        p.health = 100;
        let (dead, shown) = break_with(&mut p, 40);
        assert!(!dead);
        assert_eq!(p.health, 60);
        assert_eq!(shown, Visibility::Inherited);
    }

    #[test]
    fn clutter_cannot_be_broken() {
        let mut p = piece(DynEntType::Clutter, 1.0);
        p.health = 10;
        let (dead, shown) = break_with(&mut p, 500);
        assert!(!dead);
        assert_eq!(p.health, 10);
        assert_eq!(shown, Visibility::Inherited);
    }

    #[test]
    fn a_broken_piece_stays_broken_quietly() {
        let mut p = piece(DynEntType::Destruct, 1.0);
        p.health = 0;
        p.dead = true;
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
            ev::GRENADE_EXPLODE,
            ev::ROCKET_EXPLODE,
            ev::ROCKET_EXPLODE_NOMARKS,
            ev::FLASHBANG_EXPLODE,
        ] {
            assert!(is_radial_blast(i32::from(id)));
        }
        assert!(!is_radial_blast(i32::from(ev::MELEE_HIT)));
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
            world.init_resource::<DynEntPhysWorld>();
            world.init_resource::<EntityMarks>();
            world.init_resource::<Messages<DynEntPhysImpulse>>();
            Self { world }
        }

        /// Adds a piece and refiles every piece (each with a reach of 1, as a
        /// piece with no known model gets).
        fn add(&mut self, piece: WorldDynEntInstance, at: Vec3) -> Entity {
            let id = self
                .world
                .spawn((
                    piece,
                    Transform::from_translation(at),
                    Visibility::Inherited,
                    DynEntModelEntity,
                ))
                .id();
            let all: Vec<_> = self
                .world
                .query_filtered::<(Entity, &Transform), With<DynEntModelEntity>>()
                .iter(&self.world)
                .map(|(entity, at)| (entity, at.translation, 1.0))
                .collect();
            self.world.resource_mut::<WakeGrid>().refill(all);
            id
        }

        fn with_rubble<R>(&mut self, act: impl FnOnce(&mut Rubble) -> R) -> R {
            let mut state = SystemState::<Rubble>::new(&mut self.world);
            let mut rubble = state.get_mut(&mut self.world).unwrap();
            act(&mut rubble)
        }

        /// The shoves handed out since the last call, by piece.
        fn shoves(&mut self) -> HashMap<Entity, Vec3> {
            let mut sent = self.world.resource_mut::<Messages<DynEntPhysImpulse>>();
            sent.drain().map(|m| (m.entity, m.impulse)).collect()
        }

        fn health(&self, piece: Entity) -> i32 {
            self.world.get::<WorldDynEntInstance>(piece).unwrap().health
        }

        fn is_hidden(&self, piece: Entity) -> bool {
            self.world.get::<Visibility>(piece) == Some(&Visibility::Hidden)
        }
    }

    #[test]
    fn a_shot_shoves_the_first_piece_on_its_path() {
        let mut yard = Yard::new();
        let near = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(50.0, 0.0, 0.0));
        let far = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(120.0, 0.0, 0.0));
        let off_path = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(80.0, 40.0, 0.0));
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(200.0, 0.0, 0.0), 0));
        let shoves = yard.shoves();
        assert_eq!(shoves.len(), 1);
        assert_eq!(shoves[&near], Vec3::new(DYNENT_BULLET_FORCE, 0.0, 0.0));
        assert!(!shoves.contains_key(&far) && !shoves.contains_key(&off_path));
    }

    #[test]
    fn a_shot_skips_pieces_that_cannot_be_woken() {
        let mut yard = Yard::new();
        let mut broken = piece(DynEntType::Clutter, 1.0);
        broken.dead = true;
        yard.add(broken, Vec3::new(20.0, 0.0, 0.0));
        yard.add(piece(DynEntType::Invalid, 1.0), Vec3::new(30.0, 0.0, 0.0));
        let behind = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(90.0, 0.0, 0.0));
        yard.with_rubble(|r| r.strike(Vec3::ZERO, Vec3::new(200.0, 0.0, 0.0), 0));
        let shoves = yard.shoves();
        assert_eq!(shoves.keys().copied().collect::<Vec<_>>(), vec![behind]);
    }

    #[test]
    fn a_shot_that_finishes_a_breakable_piece_breaks_it() {
        let mut yard = Yard::new();
        let mut glass = piece(DynEntType::Destruct, 1.0);
        glass.health = 40;
        let glass = yard.add(glass, Vec3::new(50.0, 0.0, 0.0));
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
        let crate_ = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(0.0, 3.0, 0.0));
        let from = Vec3::new(0.0, 2.5, 0.0);
        yard.with_rubble(|r| r.strike(from, from, 0));
        let shoves = yard.shoves();
        assert_eq!(shoves[&crate_], Vec3::new(0.0, DYNENT_BULLET_FORCE, 0.0));
    }

    #[test]
    fn a_blast_shoves_and_wounds_by_distance() {
        let mut yard = Yard::new();
        let crate_ = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(50.0, 0.0, 0.0));
        let mut glass = piece(DynEntType::Destruct, 1.0);
        glass.health = 100;
        let glass = yard.add(glass, Vec3::new(0.0, 200.0, 0.0));
        let mut vase = piece(DynEntType::Destruct, 1.0);
        vase.health = 100;
        let vase = yard.add(vase, Vec3::new(0.0, 0.0, 50.0));
        let outside = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(400.0, 0.0, 0.0));
        yard.with_rubble(|r| {
            r.detonate(Blast {
                origin: Vec3::ZERO,
                authored: Vec3::ZERO,
                inner: 100.0,
                outer: 300.0,
                inner_damage: 120,
                outer_damage: 20,
            })
        });

        let shoves = yard.shoves();
        assert_eq!(shoves.len(), 3);
        assert!(!shoves.contains_key(&outside));
        // Full strength straight along x, tipped up by the lift.
        let tilt = Vec3::new(1.0, 0.0, 0.5).normalize();
        assert!((shoves[&crate_] - tilt * DYNENT_EXPLODE_FORCE).length() < 0.5);
        // Halfway out the strength halves.
        let tilt = Vec3::new(0.0, 1.0, 0.5).normalize();
        assert!((shoves[&glass] - tilt * DYNENT_EXPLODE_FORCE * 0.5).length() < 0.5);
        // Straight up already points upward, so the lift only adds to it.
        let up = Vec3::new(0.0, 0.0, 1.5).normalize();
        assert!((shoves[&vase] - up * DYNENT_EXPLODE_FORCE).length() < 0.5);

        // 20 + 100 * 0.5 = 70 off the glass; the 120 at the centre finishes the vase.
        assert_eq!(yard.health(glass), 30);
        assert!(!yard.is_hidden(glass));
        assert_eq!(yard.health(vase), 0);
        assert!(yard.is_hidden(vase));
    }

    #[test]
    fn a_blast_with_its_own_heading_pushes_that_way() {
        let mut yard = Yard::new();
        let crate_ = yard.add(piece(DynEntType::Clutter, 1.0), Vec3::new(50.0, 0.0, 0.0));
        yard.with_rubble(|r| {
            r.detonate(Blast {
                origin: Vec3::ZERO,
                authored: Vec3::new(0.0, 0.0, 1.0),
                inner: 100.0,
                outer: 300.0,
                inner_damage: 0,
                outer_damage: 0,
            })
        });
        let shoves = yard.shoves();
        assert!((shoves[&crate_] - Vec3::new(0.0, 0.0, DYNENT_EXPLODE_FORCE)).length() < 0.5);
    }

    #[test]
    fn without_weapon_tables_nothing_goes_off() {
        let mut yard = Yard::new();
        yard.add(piece(DynEntType::Destruct, 1.0), Vec3::new(10.0, 0.0, 0.0));
        assert!(yard.with_rubble(|r| r.blast_of(7, Vec3::ZERO, Vec3::ZERO).is_none()));
        assert_eq!(yard.with_rubble(|r| r.hit_damage(7)), 0);
        assert!(yard.shoves().is_empty());
    }
}
