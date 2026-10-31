//! bo2mp: Black Ops II's window shards, as its client makes, moves and draws
//! them (`GlassClient::Shatter`, `GlassOutline::Shatter/Split/HitPoint/Chip`,
//! `GlassShard::InitPhysics/TracePoint`, `GlassPhysics`, `GlassRenderer`).
//!
//! A pane's first shatter cuts it into a random mosaic of jagged cracks; the
//! hit only aims the falling pieces. Pieces along the frame may stay in it,
//! or fall a moment later. A bullet on a piece still in the frame (or lying
//! on the floor) chips or cuts it near the hit. Falling pieces either fly
//! without collision for 3 s or hit the world and lie where they stop.
//! Numbers are BO2's dvar defaults.

use crate::GlassWorldTrace;

/// `maxShardSplit`: the piece budget when the def gives none.
const MAX_SHARD_SPLIT: usize = 125;
/// `StaticFixedSizeAllocator<TempPackedOutline, 350>`.
const SHARD_SPLIT_CAP: usize = 350;
/// `maxSplitSizeRatio`.
const MAX_SPLIT_SIZE_RATIO: f32 = 20.0;
/// `shardSplitDir`, `shardSplitDir2` (degrees), `shardEdgeSize` (perimeter).
const SPLIT_DIR: (f32, f32) = (30.0, 60.0);
const SPLIT_DIR2: (f32, f32) = (10.0, 30.0);
const EDGE_SIZE: (f32, f32) = (0.02, 0.15);
/// `narrowShardRatio`.
const NARROW_RATIO: f32 = 3.0;
/// Outline vertices a piece may have, and the closest two may stand.
const MAX_VERTS: usize = 16;
const MIN_VERT_GAP: f32 = 0.1;
/// `shardShatterSizeLimitScale`; pieces one bullet may make.
const SHATTER_SIZE_LIMIT_SCALE: f32 = 0.2;
const HIT_MAX_PIECES: usize = 10;
/// `timeUntilDropRange`, `glassMaxShardLife`.
const DROP_DELAY: (f32, f32) = (0.25, 1.0);
const MAX_SHARD_LIFE: f32 = 3.0;
/// `glassExtraLinVelocity`, `glassExtraAngVelocity`, `glassLinVelMultiplier`,
/// `glassForceOriginMult`, `glassForceAttenuation`.
const EXTRA_LIN_VEL: f32 = 4.0;
const EXTRA_ANG_VEL: f32 = 4.0;
const LIN_VEL_MULT: f32 = 250.0;
const FORCE_ORIGIN_MULT: f32 = 0.5;
const FORCE_ATTENUATION: f32 = 1.0;
/// `glassForceMultiplier` (0.5); `GlassShard::AddForce` clamps a push to
/// 500; `cg_maxExplosionRadius` (exe registration: 128, "Maximum radius of
/// client-side explosions").
const FORCE_MULTIPLIER: f32 = 0.5;
const MAX_FORCE: f32 = 500.0;
const MAX_EXPLOSION_RADIUS: f32 = 128.0;
/// `phys_gravity` along `phys_gravity_dir`, `phys_dragLinear`, `phys_dragAngular`.
const GRAVITY: f32 = -800.0;
const DRAG_LINEAR: f32 = 0.03;
const DRAG_ANGULAR: f32 = 0.5;
/// `GlassPhysics::IntegrateVelocity`: angular speed cap (rad/s).
const MAX_ANG_SPEED: f32 = 16.0;
/// Rigid-body shards at once (`AddColidingShard`); the delayed list.
const MAX_COLLIDING: usize = 100;
const MAX_DELAYED: usize = 1000;
/// The shard's rigid-body friction (`InitPhysicsObj`).
const FRICTION: f32 = 0.5;
/// Not in BO2's glass code (its engine physics bounces and rests the
/// shard; those limits were not decoded): the bounce kept along the
/// contact normal (the IW engine glass's restitution) and the speed under
/// which a piece on a floor lies still.
const RESTITUTION: f32 = 0.15;
const REST_SPEED: f32 = 40.0;
/// `glassLowLodDist`: farther than this the side walls are left out.
pub const T6_GLASS_LOW_LOD_DIST: f32 = 450.0;

/// One window's shard source: its outline in its own plane and its def.
#[derive(Clone, Debug, Default)]
pub struct T6PaneShape {
    pub outline: Vec<[f32; 2]>,
    /// In-plane x, in-plane y, normal.
    pub axis: [[f32; 3]; 3],
    pub outline_origin: [f32; 3],
    pub thickness: f32,
    pub uv_scale: f32,
    pub is_planar: bool,
    pub min_shard_size: f32,
    pub max_shard_size: f32,
    pub shard_life_probability: f32,
    pub max_shards: i32,
    pub pool_shards: u32,
    pub pool_physics: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum T6ShardMotion {
    /// In the frame, or lying where it came to rest: drawn, and a bullet or
    /// a blast acts on it.
    Still,
    /// Waiting in the frame to fall (`AddDelayedPhysicsShard`).
    Delayed,
    /// Falling with no collision (`GlassPhysics`), for its life time.
    Simple,
    /// Falling with world collision until it rests.
    Colliding,
}

/// One shard.
#[derive(Clone, Debug)]
pub struct T6Shard {
    pub pane: u32,
    /// Outline about `origin` in the plane of `axis[0]`, `axis[1]`; edge `i`
    /// runs from vertex `i` to `i + 1`.
    pub outline: Vec<[f32; 2]>,
    /// Edge `i` is part of the pane's frame.
    pub frame: Vec<bool>,
    /// Where this piece sat on the pane (its texture position).
    pub uv_offset: [f32; 2],
    pub origin: [f32; 3],
    pub along: [[f32; 3]; 3],
    pub area: f32,
    pub motion: T6ShardMotion,
    pub motion_vec: [f32; 3],
    pub avel: [f32; 3],
    /// Seconds left: until a delayed piece falls, or a simple one goes.
    pub timer: f32,
}

/// One vertex of a shard's mesh, in the world.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct T6ShardVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tex_coord: [f32; 2],
}

/// Counts for the proof logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct T6ShardCounts {
    pub still: usize,
    pub delayed: usize,
    pub simple: usize,
    pub colliding: usize,
}

/// Every window's shards.
#[derive(Clone, Debug, Default)]
pub struct T6Glass {
    pub panes: Vec<T6PaneShape>,
    pub shards: Vec<T6Shard>,
    /// The world's floor (`worldBottom`: bounds minimum z + 5).
    pub floor: f32,
    clock: Option<i32>,
    dice: Rng,
    shatter_seq: u64,
    log: bool,
    /// Bumped whenever the shards change, for the draw pass.
    pub revision: u64,
}

impl T6Glass {
    /// A new map round: these windows, no shards.
    pub fn set_panes(&mut self, panes: Vec<T6PaneShape>, floor: f32) {
        *self = Self {
            panes,
            floor,
            dice: Rng::new(0x5EED_0B02),
            log: std::env::var_os("BO2MP_T6_GLASSLOG").is_some(),
            ..Self::default()
        };
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    pub fn counts(&self) -> T6ShardCounts {
        let mut c = T6ShardCounts::default();
        for s in &self.shards {
            match s.motion {
                T6ShardMotion::Still => c.still += 1,
                T6ShardMotion::Delayed => c.delayed += 1,
                T6ShardMotion::Simple => c.simple += 1,
                T6ShardMotion::Colliding => c.colliding += 1,
            }
        }
        c
    }

    /// `GlassClient::Shatter`: pane `pane` breaks into its mosaic. `hit` and
    /// `dir` aim the falling pieces (zero `dir`: random drift). A shatter
    /// that is old news (`animate` false, BO2's game-state shatter) drops
    /// the pieces that would fly with no collision right away.
    pub fn shatter(&mut self, pane: u32, hit: [f32; 3], dir: [f32; 3], seed: Option<u64>, animate: bool) -> bool {
        let Some(shape) = self.panes.get(pane as usize) else {
            return false;
        };
        if shape.outline.len() < 3 {
            return false;
        }
        self.shatter_seq = self.shatter_seq.wrapping_add(1);
        let mut rng = Rng::new(seed.unwrap_or(self.shatter_seq.wrapping_mul(0x9E37_79B9)) ^ (u64::from(pane) << 40));
        let mut outline = Outline::new(shape.outline.clone(), shape.is_planar);
        outline.dedup();
        if outline.p.len() < 3 {
            return false;
        }
        let pane_area = outline.area();
        let mut pieces = shatter_outline(&outline, shape, &mut rng);
        // `CreateShards`: `GlassShard::Create` gives nothing once the shard
        // pool is full and the pane stops there; its fall count goes by
        // the pieces it made (`InitPhysics`).
        pieces.truncate(self.pool_shards().saturating_sub(self.shards.len()));
        let k = ((pieces.len() as f32
            * shape.shard_life_probability
            * if dir == [0.0; 3] { 0.5 } else { 1.0 })
            + 0.5) as usize;
        let (axis, origin) = (shape.axis, shape.outline_origin);
        let first = self.shards.len();
        for (i, piece) in pieces.into_iter().enumerate() {
            let shard = make_shard(pane, piece, [0.0; 2], origin, axis);
            self.place(shard, i < k, hit, dir, pane_area, 1.0, &mut rng);
        }
        if !animate {
            self.shards
                .retain(|s| s.motion != T6ShardMotion::Simple || s.pane != pane);
        }
        self.revision += 1;
        if self.log {
            let made = &self.shards[first.min(self.shards.len())..];
            let count = |m| made.iter().filter(|s| s.motion == m).count();
            eprintln!(
                "bo2mp t6 glass shards: pane {pane} shattered into {} pieces (def min {} max {} life {} maxShards {}; pane area {pane_area:.0}): {} stay in the frame, {} drop late, {} fall colliding, {} fall simple",
                made.len(),
                shape_of(&self.panes, pane).min_shard_size,
                shape_of(&self.panes, pane).max_shard_size,
                shape_of(&self.panes, pane).shard_life_probability,
                shape_of(&self.panes, pane).max_shards,
                count(T6ShardMotion::Still),
                count(T6ShardMotion::Delayed),
                count(T6ShardMotion::Colliding),
                count(T6ShardMotion::Simple),
            );
            // Where the ones in the frame are (a test shot aims at them).
            let still: Vec<String> = made
                .iter()
                .filter(|s| s.motion == T6ShardMotion::Still)
                .map(|s| format!("[{:.1} {:.1} {:.1}] area {:.0}", s.origin[0], s.origin[1], s.origin[2], s.area))
                .collect();
            eprintln!("bo2mp t6 glass shards: pane {pane} in the frame: {}", still.join(", "));
        }
        let dropped = self.maintain();
        if self.log && dropped > 0 {
            eprintln!(
                "bo2mp t6 glass shards: pool of {} over 90%: maintenance dropped the {dropped} smallest, {} left",
                self.pool_shards(),
                self.shards.len()
            );
        }
        true
    }

    /// The pane is gone outright (deleted): its shards go with it.
    pub fn clear_pane(&mut self, pane: u32) {
        let before = self.shards.len();
        self.shards.retain(|s| s.pane != pane);
        if self.shards.len() != before {
            self.revision += 1;
        }
    }

    /// `GlassCl_TracePoint` -> `GlassShard::TracePoint`: a bullet from
    /// `start` to `end` cuts the first shard it meets that is still (in the
    /// frame, waiting to drop, or lying on the floor). Returns 1 on a hit.
    pub fn hit_segment(&mut self, seg_a: [f32; 3], seg_b: [f32; 3], seed: u64) -> usize {
        let delta = sub3(seg_b, seg_a);
        let len = dot3(delta, delta).sqrt();
        if !(len > 1e-4) || !seg_a.iter().chain(seg_b.iter()).all(|v| v.is_finite()) {
            return 0;
        }
        let mut nearest_hit: Option<(f32, usize, [f32; 3])> = None;
        for (i, s) in self.shards.iter().enumerate() {
            if !matches!(s.motion, T6ShardMotion::Still | T6ShardMotion::Delayed) {
                continue;
            }
            let n = s.along[2];
            let den = dot3(delta, n);
            if den.abs() < 1e-6 {
                continue;
            }
            let f = dot3(sub3(s.origin, seg_a), n) / den;
            if !(0.0..=1.0).contains(&f) || nearest_hit.is_some_and(|b| b.0 <= f) {
                continue;
            }
            let hit = add3(seg_a, scale3(delta, f));
            let local = sub3(hit, s.origin);
            let p = [dot3(local, s.along[0]), dot3(local, s.along[1])];
            if point_in_polygon(&s.outline, p) {
                nearest_hit = Some((f, i, hit));
            }
        }
        let Some((_frac, i, hit)) = nearest_hit else {
            return 0;
        };
        self.hit_shard(i, hit, scale3(delta, 1.0 / len), seed);
        1
    }

    /// `GlassShard::TracePoint` -> `GlassOutline::HitPoint`: shard `i` is cut
    /// near `hit`; pieces still on the frame stay, the rest fall.
    fn hit_shard(&mut self, i: usize, hit: [f32; 3], heading: [f32; 3], seed: u64) {
        let parent = self.shards.swap_remove(i);
        let shape = shape_of(&self.panes, parent.pane).clone();
        let mut rng = Rng::new(seed ^ 0xB0_2C11);
        let local = sub3(hit, parent.origin);
        let h = [dot3(local, parent.along[0]), dot3(local, parent.along[1])];
        let source = Outline {
            p: parent.outline.clone(),
            f: parent.frame.clone(),
        };
        let min_size = SHATTER_SIZE_LIMIT_SCALE * shape.min_shard_size;
        let pieces = hit_point(&source, h, HIT_MAX_PIECES, min_size, shape.max_shard_size, &mut rng);
        let (lo, hi) = bbox(&parent.outline);
        let diag = (sq(hi[0] - lo[0]) + sq(hi[1] - lo[1]) + sq(shape.thickness)).sqrt();
        let (mut stay, mut fall) = (0, 0);
        for piece in pieces {
            let mut shard = make_shard(parent.pane, piece, parent.uv_offset, parent.origin, parent.along);
            if shard.frame.iter().any(|&f| f) {
                shard.motion = T6ShardMotion::Still;
                self.shards.push(shard);
                stay += 1;
            } else {
                self.fall(shard, true, hit, heading, diag.max(1e-3), &mut rng);
                fall += 1;
            }
        }
        self.maintain();
        self.revision += 1;
        if self.log {
            eprintln!(
                "bo2mp t6 glass shards: pane {} shard hit at {hit:?}: {stay} stay, {fall} fall",
                parent.pane
            );
        }
    }

    /// Per-piece `GlassShard::InitPhysics`: a piece on the frame (not on its
    /// bottom edge) may stay; one that fails waits, then falls; the rest
    /// fall now.
    fn place(&mut self, mut shard: T6Shard, collide: bool, hit: [f32; 3], heading: [f32; 3], size: f32, f: f32, rng: &mut Rng) {
        let flagged = shard.frame.iter().any(|&e| e);
        if flagged && f > 0.0 && !on_bottom_edge(&shard) {
            let o = Outline {
                p: shard.outline.clone(),
                f: shard.frame.clone(),
            };
            if leave_on_edge(&o, f, rng) {
                shard.motion = T6ShardMotion::Still;
                self.shards.push(shard);
                return;
            }
            if self.count(T6ShardMotion::Delayed) < MAX_DELAYED {
                shard.motion = T6ShardMotion::Delayed;
                shard.timer = rng.range(DROP_DELAY.0, DROP_DELAY.1);
                self.shards.push(shard);
                return;
            }
        }
        self.fall(shard, collide, hit, heading, size, rng);
    }

    /// The piece gets its body and starting velocity and falls.
    fn fall(&mut self, mut shard: T6Shard, collide: bool, hit: [f32; 3], heading: [f32; 3], size: f32, rng: &mut Rng) {
        let d2 = dot3(heading, heading);
        let away = sub3(shard.origin, hit);
        let dist = dot3(away, away).sqrt();
        let unit = if dist > 0.0 { scale3(away, 1.0 / dist) } else { [0.0; 3] };
        shard.motion_vec = if d2 == 0.0 {
            [0; 3].map(|_| rng.range(-EXTRA_LIN_VEL, EXTRA_LIN_VEL))
        } else if d2 <= 1.01 {
            let k = 1.0 - (dist * FORCE_ATTENUATION / size.max(1e-6)).clamp(0.25, 1.0);
            scale3(add3(scale3(unit, FORCE_ORIGIN_MULT), heading), k * LIN_VEL_MULT)
        } else {
            let s = ((d2.sqrt() - 1.0) * 2.0).clamp(0.2, 1.0);
            scale3(unit, s * LIN_VEL_MULT)
        };
        shard.avel = [0; 3].map(|_| rng.range(-EXTRA_ANG_VEL, EXTRA_ANG_VEL));
        // A piece out of physics bodies is gone (OUT_OF_PHYSICS).
        let _ = self.add_body(shard, collide);
    }

    /// `InitPhysicsObj`: the piece leaves the frame with its life time and
    /// a colliding body, or a simple one when `collide` is off or the
    /// colliding cap keeps it out. Gives the piece back when no simple
    /// body is left.
    fn add_body(&mut self, mut shard: T6Shard, collide: bool) -> Result<(), T6Shard> {
        shard.frame.iter_mut().for_each(|e| *e = false);
        shard.timer = MAX_SHARD_LIFE;
        shard.motion = T6ShardMotion::Colliding;
        if !collide {
            shard.motion = T6ShardMotion::Simple;
        } else if self.count(T6ShardMotion::Colliding) >= MAX_COLLIDING {
            // `AddColidingShard`: over the cap the smallest may not collide.
            let smallest = self
                .shards
                .iter()
                .enumerate()
                .filter(|entry| entry.1.motion == T6ShardMotion::Colliding)
                .min_by(|lhs, rhs| lhs.1.area.total_cmp(&rhs.1.area))
                .map(|(i, s)| (i, s.area));
            match smallest {
                Some((i, area)) if area < shard.area => {
                    self.shards[i].motion = T6ShardMotion::Simple;
                }
                _ => shard.motion = T6ShardMotion::Simple,
            }
        }
        if shard.motion == T6ShardMotion::Simple && self.count(T6ShardMotion::Simple) >= self.pool_physics() {
            return Err(shard);
        }
        self.shards.push(shard);
        Ok(())
    }

    /// `CG_ExplosionEvent` -> `GlassShard::ExplosionEvent`: every piece
    /// still in a frame, waiting to drop or lying where it came to rest,
    /// closer than `radius` (capped by `cg_maxExplosionRadius`) to the
    /// centre, gets a body (same cap as a fall) at rest and one push away
    /// from the centre: `inner` strength at the centre falling to `outer`
    /// at the edge, times 0.01 and `glassForceMultiplier`, at most 500
    /// (`AddForce`), over its mass. Its frame hold is gone either way.
    /// Returns how many pieces it knocked.
    pub fn explosion(&mut self, centre: [f32; 3], inner: f32, outer: f32, reach: f32) -> usize {
        let reach = reach.min(MAX_EXPLOSION_RADIUS);
        if !(reach > 0.0) || !centre.iter().all(|v| v.is_finite()) {
            return 0;
        }
        let near = |s: &T6Shard| {
            let away = sub3(s.origin, centre);
            matches!(s.motion, T6ShardMotion::Still | T6ShardMotion::Delayed) && dot3(away, away) < reach * reach
        };
        let (hit, keep): (Vec<T6Shard>, Vec<T6Shard>) = std::mem::take(&mut self.shards).into_iter().partition(near);
        self.shards = keep;
        if hit.is_empty() {
            return 0;
        }
        let knocked = hit.len();
        let mut stuck = 0;
        for mut shard in hit {
            let away = sub3(shard.origin, centre);
            let d = dot3(away, away).sqrt();
            let unit = if d > 0.0 { scale3(away, 1.0 / d) } else { away };
            let push = ((1.0 - d / reach) * (inner - outer) + outer) * 0.01 * FORCE_MULTIPLIER;
            let mut force = scale3(unit, push);
            let len = dot3(force, force).sqrt();
            if len > MAX_FORCE {
                force = scale3(force, MAX_FORCE / len);
            }
            // The push lands on the piece's own origin: no spin. Both body
            // kinds take it as one step's impulse, force / mass.
            let mass = mass_of(shard.area, shape_of(&self.panes, shard.pane).thickness);
            shard.motion_vec = scale3(force, 1.0 / mass);
            shard.avel = [0.0; 3];
            if let Err(mut left) = self.add_body(shard, true) {
                // No body: it stays where it is, its frame hold gone.
                left.motion = T6ShardMotion::Still;
                left.motion_vec = [0.0; 3];
                self.shards.push(left);
                stuck += 1;
            }
        }
        self.revision += 1;
        if self.log {
            let c = self.counts();
            eprintln!(
                "bo2mp t6 glass shards: blast at {centre:?} radius {reach} ({inner} to {outer}) knocked {knocked} pieces ({stuck} with no body; now {} still, {} delayed, {} colliding, {} simple)",
                c.still, c.delayed, c.colliding, c.simple
            );
        }
        knocked
    }

    fn count(&self, m: T6ShardMotion) -> usize {
        self.shards.iter().filter(|s| s.motion == m).count()
    }

    fn pool_physics(&self) -> usize {
        self.panes.iter().map(|p| p.pool_physics).max().filter(|&n| n > 0).unwrap_or(512) as usize
    }

    fn pool_shards(&self) -> usize {
        self.panes.iter().map(|p| p.pool_shards).max().filter(|&n| n > 0).unwrap_or(2048) as usize
    }

    /// `DoMaintenance` (b): over 90% of the shard pool, the smallest go.
    /// BO2 skips shards in view; this skips pieces still in a frame.
    /// Returns how many went.
    fn maintain(&mut self) -> usize {
        let pool = self.pool_shards();
        if self.shards.len() * 10 <= pool * 9 {
            return 0;
        }
        let keep = ((pool as f32 * 0.9 - 5.0).round().max(0.0)) as usize;
        let mut order: Vec<usize> = (0..self.shards.len())
            .filter(|&i| !self.shards[i].frame.iter().any(|&f| f))
            .collect();
        order.sort_by(|&a, &b| self.shards[a].area.total_cmp(&self.shards[b].area));
        let drop = self.shards.len().saturating_sub(keep).min(order.len());
        let mut gone = vec![false; self.shards.len()];
        for &i in &order[..drop] {
            gone[i] = true;
        }
        let mut i = 0;
        self.shards.retain(|_| {
            i += 1;
            !gone[i - 1]
        });
        drop
    }

    /// One frame (`GlassRenderer::Update`): delayed pieces count down and
    /// fall, then every falling piece steps once (dt clamped to 0.1 s).
    pub fn advance(&mut self, msec: i32, world: Option<&dyn GlassWorldTrace>) {
        let Some(last) = self.clock.replace(msec) else {
            return;
        };
        let dt = ((msec - last) as f32 * 0.001).clamp(0.0, 0.1);
        if dt <= 0.0 || self.shards.is_empty() {
            return;
        }
        let mut kept_dice = std::mem::take(&mut self.dice);
        let mut i = 0;
        while i < self.shards.len() {
            if self.shards[i].motion == T6ShardMotion::Delayed {
                self.shards[i].timer -= dt;
                if self.shards[i].timer < 0.0 {
                    let shard = self.shards.swap_remove(i);
                    self.fall(shard, true, [0.0; 3], [0.0; 3], 0.0, &mut kept_dice);
                    continue;
                }
            }
            i += 1;
        }
        self.dice = kept_dice;
        let floor = self.floor;
        let panes = &self.panes;
        let mut rested = 0;
        self.shards.retain_mut(|s| match s.motion {
            T6ShardMotion::Simple => {
                step_body(s, dt);
                s.timer -= dt;
                s.timer >= 0.0 && s.origin[2] >= floor
            }
            T6ShardMotion::Colliding => {
                let from = s.origin;
                step_body(s, dt);
                if let Some(world) = world {
                    if collide(s, from, world, shape_of(panes, s.pane).thickness) {
                        rested += 1;
                    }
                }
                s.origin[2] >= floor
            }
            _ => true,
        });
        if self.log && rested > 0 {
            let c = self.counts();
            eprintln!(
                "bo2mp t6 glass shards: {rested} came to rest ({} still, {} delayed, {} colliding, {} simple)",
                c.still, c.delayed, c.colliding, c.simple
            );
        }
        self.revision += 1;
    }

    /// Shard `i`'s mesh (`Mesh::Init`, `GlassShard::GenerateVerts`): front
    /// face on the outline, back face `thickness` along the normal, and
    /// (high LOD) one quad per edge. UVs are where it sat on the pane.
    pub fn shard_mesh(&self, i: usize, walls: bool, vertex_points: &mut Vec<T6ShardVertex>, indices: &mut Vec<u32>) {
        let Some(s) = self.shards.get(i) else {
            return;
        };
        let shape = shape_of(&self.panes, s.pane);
        let n = s.outline.len();
        if n < 3 {
            return;
        }
        let [ax, ay, az] = s.along;
        let back = scale3(az, shape.thickness);
        let world = |p: [f32; 2]| add3(s.origin, add3(scale3(ax, p[0]), scale3(ay, p[1])));
        let uv = |p: [f32; 2]| [(s.uv_offset[0] + p[0]) * shape.uv_scale, (s.uv_offset[1] + p[1]) * shape.uv_scale];
        let triangle_list = triangulate(&s.outline);
        // Front faces +normal, back faces -normal; each wound to face out.
        let handed = dot3(cross3(ax, ay), az) >= 0.0;
        let ccw = signed_area(&s.outline) >= 0.0;
        let front_flip = handed != ccw;
        let base = vertex_points.len() as u32;
        for &p in &s.outline {
            vertex_points.push(T6ShardVertex { position: world(p), normal: az, tex_coord: uv(p) });
        }
        for &p in &s.outline {
            vertex_points.push(T6ShardVertex { position: add3(world(p), back), normal: scale3(az, -1.0), tex_coord: uv(p) });
        }
        let nb = base + n as u32;
        for &[a, b, c] in &triangle_list {
            let (a, b, c) = (a as u32, b as u32, c as u32);
            if front_flip {
                indices.extend([base + a, base + c, base + b, nb + a, nb + b, nb + c]);
            } else {
                indices.extend([base + a, base + b, base + c, nb + a, nb + c, nb + b]);
            }
        }
        if !walls {
            return;
        }
        for e in 0..n {
            let (p0, p1) = (s.outline[e], s.outline[(e + 1) % n]);
            let d = sub2(p1, p0);
            // Outward in the plane: right of the edge for a CCW outline.
            let out2 = if ccw { [d[1], -d[0]] } else { [-d[1], d[0]] };
            let normal = norm3(add3(scale3(ax, out2[0]), scale3(ay, out2[1])));
            let w = vertex_points.len() as u32;
            let (f0, f1) = (world(p0), world(p1));
            for (pos, p) in [(f0, p0), (f1, p1), (add3(f1, back), p1), (add3(f0, back), p0)] {
                vertex_points.push(T6ShardVertex { position: pos, normal, tex_coord: uv(p) });
            }
            indices.extend([w, w + 1, w + 2, w, w + 2, w + 3]);
        }
    }
}

fn shape_of(panes: &[T6PaneShape], pane: u32) -> &T6PaneShape {
    static EMPTY: std::sync::OnceLock<T6PaneShape> = std::sync::OnceLock::new();
    panes
        .get(pane as usize)
        .unwrap_or_else(|| EMPTY.get_or_init(T6PaneShape::default))
}

/// A piece recentred on its box (`Recenter`): its origin moves in its
/// plane and its texture offset carries the move.
fn make_shard(pane: u32, o: Outline, uv_offset: [f32; 2], shard_at: [f32; 3], along: [[f32; 3]; 3]) -> T6Shard {
    let (lo, hi) = bbox(&o.p);
    let c = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    let area = o.area();
    T6Shard {
        pane,
        outline: o.p.iter().map(|&p| sub2(p, c)).collect(),
        frame: o.f,
        uv_offset: add2(uv_offset, c),
        origin: add3(shard_at, add3(scale3(along[0], c[0]), scale3(along[1], c[1]))),
        along,
        area,
        motion: T6ShardMotion::Still,
        motion_vec: [0.0; 3],
        avel: [0.0; 3],
        timer: 0.0,
    }
}

/// `GlassShard::GetMass`: thickness times area, under 100 weighs 0.001,
/// over 1000 weighs 0.01, straight between.
fn mass_of(area: f32, thickness: f32) -> f32 {
    let v = thickness.abs() * area;
    if v < 100.0 {
        0.001
    } else if v > 1000.0 {
        0.01
    } else {
        (v - 100.0) / 900.0 * 0.009 + 0.001
    }
}

/// `IsOnBottomEdge`: an upright piece whose one frame edge is level and
/// lowest falls at once.
fn on_bottom_edge(s: &T6Shard) -> bool {
    let [ax, ay, az] = s.along;
    if az[2].abs() >= 0.01 {
        return false;
    }
    let flagged: Vec<usize> = (0..s.frame.len()).filter(|&i| s.frame[i]).collect();
    let [e] = flagged[..] else {
        return false;
    };
    let n = s.outline.len();
    let z = |p: [f32; 2]| ax[2] * p[0] + ay[2] * p[1];
    let (p0, p1) = (s.outline[e], s.outline[(e + 1) % n]);
    let d = sub2(p1, p0);
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len <= 0.0 || ((ax[2] * d[0] + ay[2] * d[1]) / len).abs() >= 0.01 {
        return false;
    }
    let low = z(p0).max(z(p1));
    s.outline.iter().all(|&p| z(p) >= low - 0.01)
}

/// `GlassPhysics::IntegrateVelocity` + `Step` for one dt.
fn step_body(s: &mut T6Shard, dt: f32) {
    s.motion_vec[2] += GRAVITY * dt;
    // Gyroscopic term in body axes, solid box inertia.
    let (lo, hi) = bbox(&s.outline);
    let l = [hi[0] - lo[0], hi[1] - lo[1], 0.0f32];
    let inertia = [sq(l[1]) + sq(l[2]), sq(l[0]) + sq(l[2]), sq(l[0]) + sq(l[1])];
    let wb = [dot3(s.avel, s.along[0]), dot3(s.avel, s.along[1]), dot3(s.avel, s.along[2])];
    let mut nb = wb;
    for k in 0..3 {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        if inertia[k] > 1e-6 {
            nb[k] += dt * (inertia[k1] - inertia[k2]) / inertia[k] * wb[k1] * wb[k2];
        }
    }
    s.avel = add3(add3(scale3(s.along[0], nb[0]), scale3(s.along[1], nb[1])), scale3(s.along[2], nb[2]));
    let w2 = dot3(s.avel, s.avel);
    if w2 > MAX_ANG_SPEED * MAX_ANG_SPEED {
        s.avel = scale3(s.avel, MAX_ANG_SPEED / w2.sqrt());
    }
    s.motion_vec = scale3(s.motion_vec, 1.0 - (DRAG_LINEAR * dt).min(0.9));
    s.avel = scale3(s.avel, 1.0 - (DRAG_ANGULAR * dt).min(0.9));
    s.origin = add3(s.origin, scale3(s.motion_vec, dt));
    let turn = scale3(s.avel, dt);
    s.along = s.along.map(|a| rotate3(a, turn));
    orthonormalize(&mut s.along);
}

/// A colliding piece meets the world along its step: it bounces off with
/// friction, and on a floor at low speed it lies flat and stays (BO2's
/// engine rest). Glass is passed through. True when it came to rest.
fn collide(s: &mut T6Shard, from: [f32; 3], world: &dyn GlassWorldTrace, thickness: f32) -> bool {
    let Some(hit) = world.sweep(from, s.origin) else {
        return false;
    };
    if hit.began_inside || hit.pane.is_some() {
        return false;
    }
    let n = norm3(hit.normal);
    s.origin = add3(hit.end, scale3(n, 0.1));
    let normal_speed = dot3(s.motion_vec, n);
    if normal_speed < 0.0 {
        let normal_change = -normal_speed * (1.0 + RESTITUTION);
        let tangent = sub3(s.motion_vec, scale3(n, normal_speed));
        let tlen = dot3(tangent, tangent).sqrt();
        let cut = (FRICTION * normal_change).min(tlen);
        let tangent = if tlen > 0.0 { scale3(tangent, (tlen - cut) / tlen) } else { tangent };
        s.motion_vec = add3(tangent, scale3(n, -normal_speed * RESTITUTION));
        s.avel = scale3(s.avel, 0.5);
    }
    if n[2] > 0.7 && dot3(s.motion_vec, s.motion_vec) < REST_SPEED * REST_SPEED {
        // Lie flat on the floor, the side nearest it down (the back face
        // sits `thickness` along the normal, so a face-up piece lifts by it).
        let face_up = dot3(s.along[2], n) < 0.0;
        let up = if face_up { scale3(n, -1.0) } else { n };
        let handed = dot3(cross3(s.along[0], s.along[1]), s.along[2]) >= 0.0;
        let x = sub3(s.along[0], scale3(up, dot3(s.along[0], up)));
        let x = if dot3(x, x) > 1e-6 { norm3(x) } else { perpendicular(up) };
        let y = if handed { cross3(up, x) } else { cross3(x, up) };
        s.along = [x, y, up];
        let lift = if face_up { thickness.abs() } else { 0.0 };
        s.origin = add3(hit.end, scale3(n, 0.1 + lift));
        s.motion_vec = [0.0; 3];
        s.avel = [0.0; 3];
        s.motion = T6ShardMotion::Still;
        return true;
    }
    false
}

// ---- Outlines (`GlassOutline`) ----

#[derive(Clone, Debug, PartialEq)]
struct Outline {
    p: Vec<[f32; 2]>,
    /// Edge `i` (vertex `i` to `i + 1`) is frame.
    f: Vec<bool>,
}

impl Outline {
    fn new(p: Vec<[f32; 2]>, frame: bool) -> Self {
        let f = vec![frame; p.len()];
        Self { p, f }
    }

    fn len(&self) -> usize {
        self.p.len()
    }

    fn edge(&self, i: usize) -> ([f32; 2], [f32; 2]) {
        (self.p[i], self.p[(i + 1) % self.p.len()])
    }

    fn edge_len(&self, i: usize) -> f32 {
        let (a, b) = self.edge(i);
        len2(sub2(b, a))
    }

    fn perimeter(&self) -> f32 {
        (0..self.len()).map(|i| self.edge_len(i)).sum()
    }

    fn area(&self) -> f32 {
        signed_area(&self.p).abs()
    }

    fn flagged(&self) -> bool {
        self.f.iter().any(|&f| f)
    }

    fn clear(&mut self) {
        self.f.iter_mut().for_each(|f| *f = false);
    }

    /// `GetEdgeRatio`: frame length over perimeter.
    fn edge_ratio(&self) -> f32 {
        let per = self.perimeter();
        if per <= 0.0 {
            return 0.0;
        }
        (0..self.len()).filter(|&i| self.f[i]).map(|i| self.edge_len(i)).sum::<f32>() / per
    }

    /// `Add` / `CloseOutline`: a vertex within 0.1 of the one before goes,
    /// and so does a closing edge shorter than 0.1. The edge left runs on
    /// as the dropped vertex's did, so it keeps that edge's frame flag.
    fn dedup(&mut self) {
        let mut p = Vec::with_capacity(self.p.len());
        let mut f: Vec<bool> = Vec::with_capacity(self.p.len());
        for (i, &v) in self.p.iter().enumerate() {
            if let Some(&last) = p.last()
                && len2(sub2(v, last)) < MIN_VERT_GAP
            {
                if let Some(flag) = f.last_mut() {
                    *flag = self.f[i];
                }
                continue;
            }
            p.push(v);
            f.push(self.f[i]);
        }
        while p.len() > 1 && len2(sub2(p[p.len() - 1], p[0])) < MIN_VERT_GAP {
            p.pop();
            f.pop();
        }
        self.p = p;
        self.f = f;
    }
}

/// `GlassOutline::Shatter`: cut the biggest piece until all are small
/// enough or the budget is spent.
fn shatter_outline(pane: &Outline, shape: &T6PaneShape, rng: &mut Rng) -> Vec<Outline> {
    let pane_area = pane.area();
    let max_size = shape.max_shard_size.min(pane_area * 0.1);
    let min_size = shape.min_shard_size.min(max_size * 0.2);
    let budget = if shape.max_shards > 0 { shape.max_shards as usize } else { MAX_SHARD_SPLIT }.min(SHARD_SPLIT_CAP);
    let mut pieces = vec![(pane.clone(), pane_area)];
    let skip = (2.5 * min_size).min(max_size);
    let mut attempts = 0;
    while pieces.len() + 1 < budget && attempts < budget * 3 {
        let largest = pieces[0].1;
        if !(largest > max_size) {
            break;
        }
        let min_piece = min_size.max(largest / MAX_SPLIT_SIZE_RATIO);
        let mut split = None;
        let mut any = false;
        for i in 0..pieces.len() {
            if pieces[i].1 <= skip {
                continue;
            }
            if attempts >= budget * 3 {
                break;
            }
            any = true;
            attempts += 1;
            if let Some(halves) = split_random(&pieces[i].0, min_piece, rng) {
                split = Some((i, halves));
                break;
            }
        }
        match split {
            Some((i, (a, b))) => {
                pieces.remove(i);
                for o in [a, b] {
                    let area = o.area();
                    let key = sort_key(area);
                    let at = pieces.iter().position(|q| sort_key(q.1) < key).unwrap_or(pieces.len());
                    pieces.insert(at, (o, area));
                }
            }
            None if !any => break,
            None => {}
        }
    }
    pieces.into_iter().map(|(o, _)| o).collect()
}

/// `TempPackedOutline::Init`: pieces sort by their area as a 16-bit key.
fn sort_key(area: f32) -> u16 {
    area as i32 as u16
}

/// `GlassOutline::Split`, random mode: a crack from a random point on a
/// long-enough edge, into the piece.
fn split_random(o: &Outline, min_piece: f32, rng: &mut Rng) -> Option<(Outline, Outline)> {
    let per = o.perimeter();
    let n = o.len();
    let mut pick = None;
    for _ in 0..10 {
        let mut d = rng.unit() * per;
        for i in 0..n {
            let l = o.edge_len(i);
            if d <= l || i == n - 1 {
                if l >= 0.025 * per {
                    pick = Some((i, (d / l).clamp(0.25, 0.75)));
                }
                break;
            }
            d -= l;
        }
        if pick.is_some() {
            break;
        }
    }
    let (e, t) = pick?;
    let (a, b) = o.edge(e);
    let start = lerp2(a, b, t);
    let inward = inward_normal(o, norm2(sub2(b, a)));
    let dir = rotate2(inward, rng.sign() * rng.range(SPLIT_DIR.0, SPLIT_DIR.1));
    crack(o, e, start, dir, min_piece, rng)
}

/// `GlassOutline::Split`, hit mode: from a given edge point, up to ten
/// headings of 30..150 degrees off the edge.
fn split_hit(o: &Outline, e: usize, t: f32, min_piece: f32, rng: &mut Rng) -> Option<(Outline, Outline)> {
    let (a, b) = o.edge(e);
    let start = lerp2(a, b, t);
    let along = norm2(sub2(b, a));
    for _ in 0..10 {
        let dir = rotate2(along, rng.sign() * rng.range(30.0, 150.0));
        if let Some(halves) = crack(o, e, start, dir, min_piece, rng) {
            return Some(halves);
        }
    }
    None
}

/// One jagged crack from `start` (on edge `e`) heading `dir`: segments of
/// 2..15% of the perimeter, bending 10..30 degrees each way in turn, to the
/// far wall. Both halves must pass `InitSplitOutlines`.
fn crack(o: &Outline, e: usize, crack_at: [f32; 2], dir: [f32; 2], min_piece: f32, rng: &mut Rng) -> Option<(Outline, Outline)> {
    let per = o.perimeter();
    let min_cut = EDGE_SIZE.0 * per;
    let (count, mut wall_t, mut wall_e) = ray(o, crack_at, dir, Some(e));
    if count % 2 == 0 || wall_t <= min_cut {
        return None;
    }
    let mut pts = vec![crack_at];
    let (mut cur, mut dir) = (crack_at, dir);
    let mut turn_way = rng.sign();
    loop {
        let seg = rng.range(EDGE_SIZE.0 * per, EDGE_SIZE.1 * per);
        if wall_t - seg < 0.5 * min_cut {
            pts.push(add2(cur, scale2(dir, wall_t)));
            break;
        }
        cur = add2(cur, scale2(dir, seg));
        pts.push(cur);
        if pts.len() >= 62 {
            return None;
        }
        let mut next = None;
        for _ in 0..5 {
            let d = rotate2(dir, turn_way * rng.range(SPLIT_DIR2.0, SPLIT_DIR2.1));
            let (count, t, w) = ray(o, cur, d, None);
            if count % 2 == 1 && !crosses_polyline(cur, add2(cur, scale2(d, t)), &pts[..pts.len() - 1]) {
                next = Some((d, t, w));
                break;
            }
        }
        turn_way = -turn_way;
        let (d, t, w) = next?;
        dir = d;
        wall_t = t;
        wall_e = w;
    }
    halves(o, e, wall_e, &pts, false, min_piece)
}

/// `GlassOutline::Chip`: leg one from the nearest edge point to the hit,
/// leg two bent 30..60 degrees to the first wall.
fn chip(o: &Outline, e: usize, edge_point: [f32; 2], hit: [f32; 2], min_piece: f32, rng: &mut Rng) -> Option<(Outline, Outline)> {
    let leg = sub2(hit, edge_point);
    if len2(leg) < MIN_VERT_GAP {
        return None;
    }
    let d = rotate2(norm2(leg), rng.sign() * rng.range(30.0, 60.0));
    let (count, t, w) = ray(o, hit, d, None);
    if count == 0 || !t.is_finite() {
        return None;
    }
    let wall = add2(hit, scale2(d, t));
    halves(o, e, w, &[edge_point, hit, wall], true, min_piece)
}

/// The two outlines either side of a crack `pts` from edge `e` to edge
/// `w`, with frame flags copied from the cut edges (crack edges never
/// frame), checked as `InitSplitOutlines` checks them.
fn halves(o: &Outline, e: usize, w: usize, cut_pts: &[[f32; 2]], chip: bool, min_piece: f32) -> Option<(Outline, Outline)> {
    let n = o.len();
    if e == w || cut_pts.len() < 2 {
        return None;
    }
    let last = cut_pts.len() - 1;
    let mut a = Outline { p: Vec::new(), f: Vec::new() };
    for (i, &p) in cut_pts.iter().enumerate() {
        a.p.push(p);
        a.f.push(i == last && o.f[w]);
    }
    let mut j = (w + 1) % n;
    loop {
        a.p.push(o.p[j]);
        a.f.push(o.f[j]);
        if j == e {
            break;
        }
        j = (j + 1) % n;
    }
    let mut b = Outline { p: Vec::new(), f: Vec::new() };
    for (i, &p) in cut_pts.iter().rev().enumerate() {
        b.p.push(p);
        b.f.push(i == last && !chip && o.f[e]);
    }
    let mut j = (e + 1) % n;
    loop {
        b.p.push(o.p[j]);
        b.f.push(o.f[j]);
        if j == w {
            break;
        }
        j = (j + 1) % n;
    }
    a.dedup();
    b.dedup();
    (accept(&a, min_piece) && accept(&b, min_piece)).then_some((a, b))
}

/// `InitSplitOutlines` / `CloseOutline` / `HasNarrowAngle` / `IsNarrow`.
fn accept(o: &Outline, min_piece: f32) -> bool {
    let n = o.len();
    if !(3..=MAX_VERTS).contains(&n) {
        return false;
    }
    let area = o.area();
    if !(area >= min_piece) || area <= 0.0 {
        return false;
    }
    for i in 0..n {
        let d0 = norm2(sub2(o.p[(i + 1) % n], o.p[i]));
        let d1 = norm2(sub2(o.p[(i + 2) % n], o.p[(i + 1) % n]));
        if dot2(d0, d1) < -0.9 {
            return false;
        }
    }
    let (lo, hi) = bbox(&o.p);
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    let (long, short) = (w.max(h), w.min(h));
    short > 0.0 && long / short <= NARROW_RATIO && w * h / area <= NARROW_RATIO
}

/// `LeaveOnEdge`: a frame piece stays with chance (ratio - 0.15) * f * 5.
fn leave_on_edge(o: &Outline, f: f32, rng: &mut Rng) -> bool {
    if !o.flagged() {
        return false;
    }
    rng.unit() < (o.edge_ratio() - 0.15) * f * 5.000_000_5
}

/// `GlassOutline::HitPoint`: the cut a bullet makes in a shard at `hit`.
fn hit_point(o: &Outline, hit: [f32; 2], max_pieces: usize, min_size: f32, max_size: f32, rng: &mut Rng) -> Vec<Outline> {
    let per = o.perimeter();
    let skip = (2.5 * min_size).min(max_size);
    let (e, t, edge_point, dist) = nearest_edge_point(o, hit);
    let mut pieces: Vec<Outline> = Vec::new();
    if dist < 0.02 * per {
        if o.area() > skip {
            for _ in 0..5 {
                if let Some((a, b)) = split_hit(o, e, t, min_size, rng) {
                    pieces = vec![a, b];
                    break;
                }
            }
        }
    } else if let Some((a, b)) = chip(o, e, edge_point, hit, min_size, rng) {
        pieces = vec![a, b];
    }
    if pieces.is_empty() {
        pieces.push(o.clone());
    }
    for p in &mut pieces {
        if p.edge_ratio() < 0.15 {
            p.clear();
        }
    }
    let mut kept: Option<usize> = None;
    if pieces.len() == 1 {
        if pieces[0].flagged() {
            if leave_on_edge(&pieces[0], 1.0, rng) {
                kept = Some(0);
            } else {
                pieces[0].clear();
            }
        }
    } else {
        let (fa, fb) = (pieces[0].flagged(), pieces[1].flagged());
        let (a0, a1) = (pieces[0].area(), pieces[1].area());
        if fa && fb {
            let small = usize::from(a1 < a0);
            pieces[small].clear();
            kept = Some(1 - small);
        } else if fa || fb {
            let i = usize::from(fb);
            if leave_on_edge(&pieces[i], 1.0, rng) {
                kept = Some(i);
            } else {
                pieces[i].clear();
            }
        }
        if a0 > 3.0 * a1 {
            kept = Some(0);
        } else if a1 > 3.0 * a0 {
            kept = Some(1);
        }
    }
    // "Split 2": cut the others at their point nearest the hit.
    let mut list: Vec<(Outline, bool)> = pieces
        .into_iter()
        .enumerate()
        .map(|(i, p)| (p, kept == Some(i)))
        .collect();
    let mut tries = 0;
    'cut: while list.len() < max_pieces && tries < 3 * max_pieces {
        let mut any = false;
        for i in 0..list.len() {
            if list[i].1 || list[i].0.area() <= skip {
                continue;
            }
            if tries >= 3 * max_pieces {
                break 'cut;
            }
            any = true;
            tries += 1;
            let (e, t, _, _) = nearest_edge_point(&list[i].0, hit);
            if let Some((a, b)) = split_hit(&list[i].0, e, t, min_size, rng) {
                list[i] = (a, false);
                list.push((b, false));
                list.sort_by(|x, y| y.0.area().total_cmp(&x.0.area()));
                continue 'cut;
            }
        }
        if !any {
            break;
        }
    }
    list.into_iter().map(|(p, _)| p).collect()
}

/// The outline point nearest `p`: edge, parameter, point, distance.
fn nearest_edge_point(o: &Outline, p: [f32; 2]) -> (usize, f32, [f32; 2], f32) {
    let mut best = (0, 0.0, o.p[0], f32::MAX);
    for i in 0..o.len() {
        let (a, b) = o.edge(i);
        let d = sub2(b, a);
        let l2 = dot2(d, d);
        let t = if l2 > 0.0 { (dot2(sub2(p, a), d) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q = lerp2(a, b, t);
        let dist = len2(sub2(p, q));
        if dist < best.3 {
            best = (i, t, q, dist);
        }
    }
    best
}

/// A ray against the outline: crossings, distance to the first, its edge.
fn ray(o: &Outline, from: [f32; 2], dir: [f32; 2], skip: Option<usize>) -> (usize, f32, usize) {
    let mut count = 0;
    let mut best = (f32::INFINITY, 0);
    for i in 0..o.len() {
        if Some(i) == skip {
            continue;
        }
        let (a, b) = o.edge(i);
        let ab = sub2(b, a);
        let den = cross2(dir, ab);
        if den.abs() < 1e-9 {
            continue;
        }
        let ap = sub2(a, from);
        let t = cross2(ap, ab) / den;
        let s = cross2(ap, dir) / den;
        if t > 1e-4 && (0.0..1.0).contains(&s) {
            count += 1;
            if t < best.0 {
                best = (t, i);
            }
        }
    }
    (count, best.0, best.1)
}

/// `DoesIntersect`: the segment p..q crosses the polyline.
fn crosses_polyline(p: [f32; 2], q: [f32; 2], line: &[[f32; 2]]) -> bool {
    line.windows(2).any(|w| segments_cross(p, q, w[0], w[1]))
}

fn segments_cross(p: [f32; 2], q: [f32; 2], a: [f32; 2], b: [f32; 2]) -> bool {
    let r = sub2(q, p);
    let s = sub2(b, a);
    let den = cross2(r, s);
    if den.abs() < 1e-9 {
        return false;
    }
    let ap = sub2(a, p);
    let t = cross2(ap, s) / den;
    let u = cross2(ap, r) / den;
    t > 1e-4 && t < 1.0 - 1e-4 && u > 1e-4 && u < 1.0 - 1e-4
}

fn inward_normal(o: &Outline, along: [f32; 2]) -> [f32; 2] {
    if signed_area(&o.p) >= 0.0 { [-along[1], along[0]] } else { [along[1], -along[0]] }
}

fn point_in_polygon(poly: &[[f32; 2]], pt: [f32; 2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > pt[1]) != (b[1] > pt[1]) && pt[0] < (b[0] - a[0]) * (pt[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Ear clipping: `n - 2` triangles for any simple outline (a fan when the
/// clip gets stuck).
fn triangulate(poly: &[[f32; 2]]) -> Vec<[usize; 3]> {
    let n = poly.len();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return out;
    }
    let ccw = signed_area(poly) >= 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (i0, i1, i2) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (poly[i0], poly[i1], poly[i2]);
            let turn = cross2(sub2(b, a), sub2(c, b));
            if (turn > 0.0) != ccw || turn == 0.0 {
                continue;
            }
            let blocked = idx.iter().any(|&j| {
                j != i0 && j != i1 && j != i2 && point_in_triangle(poly[j], a, b, c)
            });
            if blocked {
                continue;
            }
            out.push([i0, i1, i2]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    } else {
        for k in 1..idx.len() - 1 {
            out.push([idx[0], idx[k], idx[k + 1]]);
        }
    }
    out
}

fn point_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d1 = cross2(sub2(b, a), sub2(p, a));
    let d2 = cross2(sub2(c, b), sub2(p, b));
    let d3 = cross2(sub2(a, c), sub2(p, c));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn signed_area(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    (0..n).map(|i| cross2(p[i], p[(i + 1) % n])).sum::<f32>() * 0.5
}

fn bbox(p: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for v in p {
        for k in 0..2 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    (lo, hi)
}

// ---- Small maths ----

fn sq(x: f32) -> f32 {
    x * x
}
fn add2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn scale2(v: [f32; 2], k: f32) -> [f32; 2] {
    [v[0] * k, v[1] * k]
}
fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}
fn len2(a: [f32; 2]) -> f32 {
    dot2(a, a).sqrt()
}
fn norm2(a: [f32; 2]) -> [f32; 2] {
    let l = len2(a);
    if l > 0.0 { scale2(a, 1.0 / l) } else { a }
}
fn lerp2(from: [f32; 2], to: [f32; 2], mix_t: f32) -> [f32; 2] {
    add2(from, scale2(sub2(to, from), mix_t))
}
fn rotate2(v: [f32; 2], turn_deg: f32) -> [f32; 2] {
    let (s, c) = turn_deg.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}
fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale3(v: [f32; 3], k: f32) -> [f32; 3] {
    [v[0] * k, v[1] * k, v[2] * k]
}
fn dot3(u: [f32; 3], v: [f32; 3]) -> f32 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm3(a: [f32; 3]) -> [f32; 3] {
    let l = dot3(a, a).sqrt();
    if l > 0.0 { scale3(a, 1.0 / l) } else { a }
}
fn perpendicular(n: [f32; 3]) -> [f32; 3] {
    let t = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    norm3(cross3(n, t))
}
/// `v` turned by the rotation vector `r` (axis times angle).
fn rotate3(v: [f32; 3], r: [f32; 3]) -> [f32; 3] {
    let angle = dot3(r, r).sqrt();
    if angle < 1e-9 {
        return v;
    }
    let k = scale3(r, 1.0 / angle);
    let (s, c) = angle.sin_cos();
    add3(
        add3(scale3(v, c), scale3(cross3(k, v), s)),
        scale3(k, dot3(k, v) * (1.0 - c)),
    )
}
fn orthonormalize(along: &mut [[f32; 3]; 3]) {
    let x = norm3(along[0]);
    let z = norm3(cross3(x, along[1]));
    let flip = dot3(z, along[2]) < 0.0;
    let z = if flip { scale3(z, -1.0) } else { z };
    let y = if flip { cross3(x, z) } else { cross3(z, x) };
    *along = [x, y, z];
}

/// The shard code's random numbers (uniform, `flrand`).
#[derive(Clone, Debug)]
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mix = self.0;
        mix = (mix ^ (mix >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mix = (mix ^ (mix >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mix ^ (mix >> 31)
    }
    /// 0..1.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
    fn sign(&mut self) -> f32 {
        if self.next() & 1 == 0 { 1.0 } else { -1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(w: f32, h: f32) -> T6PaneShape {
        T6PaneShape {
            outline: vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]],
            axis: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            outline_origin: [0.0, 0.0, 0.0],
            thickness: 0.5,
            uv_scale: 0.01,
            is_planar: true,
            min_shard_size: 4.0,
            max_shard_size: 40.0,
            shard_life_probability: 0.3,
            max_shards: 0,
            pool_shards: 4096,
            pool_physics: 1024,
        }
    }

    #[test]
    fn mosaic_covers_the_pane_with_valid_pieces() {
        for seed in 0..20u64 {
            let shape = pane(48.0, 64.0);
            let o = Outline::new(shape.outline.clone(), true);
            let mut rng = Rng::new(seed);
            let pieces = shatter_outline(&o, &shape, &mut rng);
            assert!(pieces.len() > 10, "seed {seed}: only {} pieces", pieces.len());
            assert!(pieces.len() < MAX_SHARD_SPLIT);
            let total: f32 = pieces.iter().map(Outline::area).sum();
            assert!((total - 48.0 * 64.0).abs() < 1.0, "seed {seed}: area {total}");
            for p in &pieces {
                assert!((3..=MAX_VERTS).contains(&p.len()));
                assert_eq!(p.p.len(), p.f.len());
            }
            // Frame edges add up to the pane border.
            let frame: f32 = pieces
                .iter()
                .map(|p| (0..p.len()).filter(|&i| p.f[i]).map(|i| p.edge_len(i)).sum::<f32>())
                .sum();
            assert!((frame - 2.0 * (48.0 + 64.0)).abs() < 1.0, "seed {seed}: frame {frame}");
        }
    }

    #[test]
    fn largest_piece_ends_small_enough() {
        let shape = pane(48.0, 64.0);
        let o = Outline::new(shape.outline.clone(), true);
        let pieces = shatter_outline(&o, &shape, &mut Rng::new(7));
        let max_size = shape.max_shard_size.min(48.0 * 64.0 * 0.1);
        let largest = pieces.iter().map(Outline::area).fold(0.0, f32::max);
        assert!(largest <= max_size * 1.0001 || pieces.len() + 1 >= MAX_SHARD_SPLIT, "largest {largest}");
    }

    #[test]
    fn bullet_on_a_frame_shard_keeps_area() {
        let shape = pane(48.0, 64.0);
        let o = Outline::new(vec![[0.0, 0.0], [12.0, 0.0], [12.0, 10.0], [0.0, 10.0]], true);
        for seed in 0..20u64 {
            let mut rng = Rng::new(seed);
            let pieces = hit_point(&o, [6.0, 5.0], HIT_MAX_PIECES, 0.2 * shape.min_shard_size, shape.max_shard_size, &mut rng);
            assert!(!pieces.is_empty() && pieces.len() <= HIT_MAX_PIECES);
            // A corner within 0.1 of the one before is dropped (BO2's
            // `Add`), so the halves may lose a sliver.
            let total: f32 = pieces.iter().map(Outline::area).sum();
            assert!((total - 120.0).abs() < 1.0, "seed {seed}: area {total}");
        }
    }

    #[test]
    fn stay_chance_follows_the_edge_ratio() {
        let mut rng = Rng::new(3);
        let all = Outline::new(vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], true);
        assert!((0..100).all(|_| leave_on_edge(&all, 1.0, &mut rng)));
        let mut one = all.clone();
        one.f = vec![false, false, false, true];
        // ratio 0.25: stays with chance 0.5.
        let stayed = (0..2000).filter(|_| leave_on_edge(&one, 1.0, &mut rng)).count();
        assert!((800..1200).contains(&stayed), "{stayed}");
        let none = Outline::new(all.p.clone(), false);
        assert!(!leave_on_edge(&none, 1.0, &mut rng));
    }

    #[test]
    fn concave_outline_triangulates_inside() {
        let l = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 1.0], [1.0, 1.0], [1.0, 4.0], [0.0, 4.0]];
        let triangle_list = triangulate(&l);
        assert_eq!(triangle_list.len(), 4);
        let area: f32 = triangle_list
            .iter()
            .map(|t| signed_area(&[l[t[0]], l[t[1]], l[t[2]]]).abs())
            .sum();
        assert!((area - 7.0).abs() < 1e-4, "{area}");
    }

    #[test]
    fn shatter_then_fall_then_settle_counts() {
        let mut g = T6Glass::default();
        g.set_panes(vec![pane(48.0, 64.0)], -1000.0);
        assert!(g.shatter(0, [0.0, 24.0, 32.0], [1.0, 0.0, 0.0], Some(11), true));
        let c = g.counts();
        assert!(c.still > 0, "{c:?}");
        assert!(c.simple + c.colliding > 0, "{c:?}");
        g.advance(0, None);
        for t in 1..=200 {
            g.advance(t * 16, None);
        }
        // Simple pieces live 3 s; with no world, colliding ones fall on.
        let c = g.counts();
        assert_eq!(c.simple, 0, "{c:?}");
        assert_eq!(c.delayed, 0, "{c:?}");
        // A bullet through a piece left in the frame cuts it.
        let s = g.shards.iter().find(|s| s.motion == T6ShardMotion::Still).unwrap().clone();
        let at = s.origin;
        let before = g.shards.len();
        let hit = g.hit_segment(add3(at, [-10.0, 0.0, 0.0]), add3(at, [10.0, 0.0, 0.0]), 5);
        assert_eq!(hit, 1);
        assert!(g.shards.len() >= before, "{} -> {}", before, g.shards.len());
    }

    #[test]
    fn shatter_stops_at_the_shard_pool() {
        // A pane makes pieces only while the shard pool has room (BO2 stops
        // at a full pool; maintenance can't cut pieces in the frame, so
        // making them all first would leave the pool overfull), and the
        // next pane gets what is left free.
        let mut shape = pane(48.0, 64.0);
        shape.pool_shards = 10;
        let mut g = T6Glass::default();
        g.set_panes(vec![shape.clone(), shape], -1000.0);
        assert!(g.shatter(0, [0.0, 24.0, 32.0], [1.0, 0.0, 0.0], Some(11), true));
        let first = g.shards.len();
        assert!(first > 0 && first <= 10, "{first}");
        assert!(g.shatter(1, [0.0, 24.0, 32.0], [1.0, 0.0, 0.0], Some(12), true));
        let second = g.shards.iter().filter(|s| s.pane == 1).count();
        assert!(g.shards.len() <= 10, "{}", g.shards.len());
        assert!(second <= 10 - first, "{second} pieces in {} free", 10 - first);
    }

    #[test]
    fn blast_pushes_near_pieces_away_within_the_capped_radius() {
        let mut g = T6Glass::default();
        g.set_panes(vec![pane(48.0, 64.0)], -1000.0);
        assert!(g.shatter(0, [0.0, 24.0, 32.0], [1.0, 0.0, 0.0], Some(11), true));
        let centre = [-120.0, 0.0, 0.0];
        let dist = |o: [f32; 3]| {
            let a = sub3(o, centre);
            dot3(a, a).sqrt()
        };
        let held = |g: &T6Glass| {
            g.shards
                .iter()
                .filter(|s| matches!(s.motion, T6ShardMotion::Still | T6ShardMotion::Delayed))
                .map(|s| s.origin)
                .collect::<Vec<_>>()
        };
        let before = held(&g);
        let near = before.iter().filter(|&&o| dist(o) < 128.0).count();
        let far = before.iter().filter(|&&o| (128.0..256.0).contains(&dist(o))).count();
        assert!(near > 0 && far > 0, "{near} near, {far} far");
        assert_eq!(g.explosion([-2000.0, 0.0, 0.0], 300.0, 75.0, 256.0), 0);
        // A 256 radius is cut to 128 on the client.
        assert_eq!(g.explosion(centre, 300.0, 75.0, 256.0), near);
        assert_eq!(held(&g).len(), before.len() - near);
        let mut seen = 0;
        let flying = |s: &&T6Shard| matches!(s.motion, T6ShardMotion::Colliding | T6ShardMotion::Simple);
        for s in g.shards.iter().filter(flying).filter(|s| s.avel == [0.0; 3]) {
            let d = dist(s.origin);
            assert!(d < 128.0, "{d}");
            assert!(s.frame.iter().all(|&f| !f));
            let push = (((1.0 - d / 128.0) * 225.0 + 75.0) * 0.005f32).min(500.0);
            let speed = dot3(s.motion_vec, s.motion_vec).sqrt();
            let want = push / mass_of(s.area, 0.5);
            assert!((speed - want).abs() < 1e-3 * want, "{speed} vs {want}");
            assert!(dot3(s.motion_vec, sub3(s.origin, centre)) > 0.0);
            seen += 1;
        }
        assert_eq!(seen, near);
    }

    #[test]
    fn mesh_counts_match_bo2() {
        let mut g = T6Glass::default();
        g.set_panes(vec![pane(48.0, 64.0)], -1000.0);
        g.shatter(0, [0.0; 3], [0.0; 3], Some(2), true);
        let n = g.shards[0].outline.len();
        let (mut v, mut i) = (Vec::new(), Vec::new());
        g.shard_mesh(0, true, &mut v, &mut i);
        assert_eq!(v.len(), 6 * n);
        assert_eq!(i.len(), 12 * (n - 1));
        let (mut v, mut i) = (Vec::new(), Vec::new());
        g.shard_mesh(0, false, &mut v, &mut i);
        assert_eq!(v.len(), 2 * n);
        assert_eq!(i.len(), 6 * (n - 2));
    }
}
