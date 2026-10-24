use crate::bullet_collision::{EntityCollisionEpoch, EntityCollisionTraceGeom};
use crate::damage::DamageAttempt;
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, PelletId};
use crate::match_state::EventAudience;
use crate::{
    BulletTraceQuery, ClientId, ColliderId, MASK_BULLET_WORLD, ProjectileId, Tick, TraceOutcome,
    bullet_trace_with_entity_models, level_time_ms,
};
use entity_iw4::{
    GLASS_PROJECTILE_PANE_HOPS, GRENADE_SPIN_PITCH_MAX, GRENADE_SPIN_PITCH_MIN,
    GRENADE_SPIN_ROLL_MAX, GRENADE_SPIN_ROLL_MIN, MISSILE_GLASS_SHATTER_VEL, MissileLandAnglesIn,
    TR_GRAVITY, TR_STATIONARY, Trajectory, evaluate_trajectory, evaluate_trajectory_delta,
    fire_grenade_no_draw_ms, fire_missile_apos, init_grenade_apos, init_grenade_pos,
    missile_land_angles,
};
use trace_iw4::surface_type_from_flags;

const SURF_TYPE_GLASS: u8 = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EquipmentRuntimeFacts {
    pub offhand_class: i32,
    /// bo2zm: Black Ops II's `offhandSlot` (1 lethal, 2 tactical), 0 = by class.
    pub offhand_slot: i32,
    pub start_ammo: i32,
    pub clip_size: i32,
    pub impact_damage: i32,
    pub fuse_time_ms: i32,

    pub hold_fire_time_ms: i32,

    pub cook_off_hold: bool,

    pub has_detonator: bool,
    pub detonate_delay_ms: i32,
    pub detonate_time_ms: i32,
    pub projectile_rotates: bool,
    pub stickiness: i32,
    pub timed_detonation: bool,
    /// The fuse waits for a floor (Black Ops 2 `bExplodeOnGround`).
    pub explode_on_ground: bool,

    pub proj_impact_explode: bool,
    /// bo2zm: Black Ops II's `bBulletImpactExplode`: its bullet explodes
    /// where it hits (`combat::bullet_impact_blast`).
    pub bullet_impact_explode: bool,
    /// bo2zm: Black Ops II's `bForceBounce`: a rocket (projectile) that
    /// bounces off what it hits; any other rocket that does not explode on
    /// impact hits and is gone (`G_FireRocket` sets the bounce flag only
    /// from this; `G_InitGrenadeEntity` always sets it for grenades).
    pub force_bounce: bool,

    pub stick_to_players: bool,
    pub ballistic_blade: bool,
    /// Black Ops II: the impact hit lands once on the same target in a row.
    pub limit_grenade_impacts: bool,
    pub explosion_radius: i32,
    pub explosion_radius_min: i32,
    pub explosion_inner_damage: i32,
    pub explosion_outer_damage: i32,
    pub damage_cone_angle: f32,
    pub missile_guidance: i32,
    pub ignition_delay_ms: i32,
    pub require_lock_to_fire: bool,
    pub projectile_speed: i32,
    pub projectile_speed_up: i32,
    pub projectile_speed_forward: i32,
    /// bo2zm: Black Ops II: extra speed along the view's up axis.
    pub projectile_speed_relative_up: i32,
    /// bo2zm: Black Ops II's parent velocity share (None on Modern Warfare 2
    /// rows). Its server only reads it for vehicles, so a throw ignores it;
    /// `Some` marks a Black Ops II row (its bounce rule).
    pub projectile_parent_velocity: Option<f32>,
    /// bo2zm fix list 2: a Black Ops II rolling grenade's radius: it rests
    /// and rolls on that much of itself instead of on its centre.
    pub rolling_radius: Option<f32>,
    pub projectile_activate_dist: i32,
    pub projectile_explosion_type: i32,
    pub weap_type: i32,
    pub weap_class: i32,
    pub parallel_bounce: Option<[f32; 31]>,
    pub perpendicular_bounce: Option<[f32; 31]>,
}

impl EquipmentRuntimeFacts {
    /// bo2zm: the class its offhand key looks for. Black Ops II gives a
    /// class-1 grenade in slot 2 (the Hell's Retriever) to the tactical key;
    /// IW4's keys pick by class, so it answers to the flash class there.
    pub fn select_class(&self) -> i32 {
        if self.offhand_slot == 2 && self.offhand_class == 1 { 3 } else { self.offhand_class }
    }

    /// bo2zm: Black Ops II's `MissileImpact` bounces a missile only when its
    /// bounce flag is set: always for a grenade (`G_InitGrenadeEntity`), and
    /// for a rocket only with `bForceBounce` (`G_FireRocket`). Any other
    /// rocket hits what it struck and is gone: the Ray Gun Mark II's bolt
    /// (no splash, no impact explode) kills one zombie and vanishes.
    pub(crate) fn rocket_ends_on_impact(self) -> bool {
        self.weap_type == weapon_iw4::WEAPTYPE_PROJECTILE && !self.force_bounce
    }
    pub fn is_usable(self) -> bool {
        (self.projectile_speed > 0 || self.stickiness != 0)
            && (self.fuse_time_ms > 0 || self.impact_damage > 0 || self.explosion_inner_damage > 0)
    }

    pub(crate) fn is_throwing_knife(self) -> bool {
        self.weap_class == WEAPCLASS_THROWINGKNIFE
    }

    pub(crate) fn is_retrievable_knife(self) -> bool {
        self.is_throwing_knife() || self.ballistic_blade
    }

    pub fn is_offhand(self) -> bool {
        self.offhand_class != 0
    }

    pub fn spawn_clip_count(self) -> i32 {
        self.start_ammo.max(self.clip_size).max(1)
    }
}

/// bo2mp: a missile stuck to a player. Black Ops II links it to the bone it
/// hit (StickMissile), so it rides along with him: here, the hit box it
/// struck and its place in that box's own axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckRider {
    pub client: ClientId,
    pub life: LifeSequence,
    /// The box's bone, or None to ride on his origin.
    pub bone: Option<u16>,
    pub local: [f32; 3],
    /// It stuck to his riot shield.
    pub shield: bool,
}

/// Where on the player it hit a missile stuck at `at` rides: the nearest box
/// of the part it hit (any box when that part has none).
fn ride_on(
    poses: &[crate::bullet_collision::PlayerCollisionPose],
    client: ClientId,
    life: LifeSequence,
    hitloc: u8,
    at: [f32; 3],
) -> Option<StuckRider> {
    let pose = poses
        .iter()
        .find(|p| p.client == client && p.life_sequence == life)?;
    let dist = |b: &&crate::bullet_collision::AuthorityDObjCollisionBone| {
        (0..3).map(|i| (b.center[i] - at[i]).powi(2)).sum::<f32>()
    };
    let nearest = |part: Option<u8>| {
        pose.bones
            .iter()
            .filter(|b| part.is_none_or(|p| b.part_classification == p))
            .min_by(|a, b| dist(a).total_cmp(&dist(b)))
    };
    let Some(bone) = nearest(Some(hitloc)).or_else(|| nearest(None)) else {
        return Some(StuckRider {
            client,
            life,
            bone: None,
            local: core::array::from_fn(|i| at[i] - pose.origin[i]),
            shield: false,
        });
    };
    let d: [f32; 3] = core::array::from_fn(|i| at[i] - bone.center[i]);
    Some(StuckRider {
        client,
        life,
        bone: Some(bone.bone),
        local: bone.axes.map(|a| (0..3).map(|i| a[i] * d[i]).sum()),
        shield: false,
    })
}

/// Where a missile riding on a player is now, or None once he is gone (dead
/// or left).
fn rider_origin(
    poses: &[crate::bullet_collision::PlayerCollisionPose],
    rider: StuckRider,
) -> Option<[f32; 3]> {
    let pose = poses
        .iter()
        .find(|p| p.client == rider.client && p.life_sequence == rider.life)?;
    let Some(id) = rider.bone else {
        return Some(core::array::from_fn(|i| pose.origin[i] + rider.local[i]));
    };
    let bone = pose.bones.iter().find(|b| b.bone == id)?;
    Some(core::array::from_fn(|i| {
        bone.center[i] + (0..3).map(|k| bone.axes[k][i] * rider.local[k]).sum::<f32>()
    }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileState {
    pub id: ProjectileId,
    pub owner: ClientId,
    pub owner_life: LifeSequence,
    pub weapon: u32,
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    pub pos: Trajectory,
    pub apos: Trajectory,
    pub entnum: i32,
    pub launch_time: i32,
    pub spawn_time_ms: i32,
    pub detonate_at_ms: Option<i32>,
    pub cleanup_at_ms: i32,
    pub travel_distance: f32,
    pub live: bool,
    pub stuck_pane: Option<u32>,
    pub grounded: bool,
    pub guide: crate::MissileGuide,
    /// The player its last impact hit landed on.
    pub impact_target: Option<ClientId>,
    /// bo2mp: the player it stuck to, and where on him.
    pub rider: Option<StuckRider>,
}

impl ProjectileState {
    pub fn origin_at(&self, at_time_ms: i32) -> [f32; 3] {
        evaluate_trajectory(&self.pos, at_time_ms)
    }

    pub fn velocity_at(&self, at_time_ms: i32) -> [f32; 3] {
        evaluate_trajectory_delta(&self.pos, at_time_ms)
    }

    pub fn is_armed(&self, activate_dist: i32) -> bool {
        self.live && (activate_dist <= 0 || self.travel_distance >= activate_dist as f32)
    }
}

pub const GRENADE_FUSE_CAP_MS: i32 = 60_000;
const OFFHAND_CLASS_SMOKE: i32 = 2;
pub const GRENADE_DEFAULT_FUSE_MS: i32 = 30_000;
pub const ROCKET_CLEANUP_MS: i32 = 60_000;
pub const BOUNCE_EVENT_SPEED_DELTA: f32 = 100.0;
const WEAPCLASS_THROWINGKNIFE: i32 = 9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GrenadeLaunchKind {
    Thrown { remaining_fuse_ms: Option<i32> },
    Launcher,
    /// bo2zm: cooked too long: it goes off in his hand (dropped where his
    /// hand is, no fuse left), the whole explosion: effect, sound, damage.
    InHand,
    /// bo2zm: a script's `magicgrenadetype`: its own velocity and fuse.
    Magic { fuse_ms: i32 },
    /// BO2's death drop: the grenade a dying player held, let go with its own velocity and
    /// what was left of its fuse (spent = goes off on the next frame).
    Dropped { remaining_fuse_ms: i32, velocity: [f32; 3] },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WeaponNote {
    Fired {
        owner: ClientId,
    },
    Detonated {
        id: crate::ProjectileId,
        origin: [f32; 3],
    },
    Stuck {
        id: crate::ProjectileId,
        /// The player it stuck to.
        player: Option<ClientId>,
        /// It stuck to the riot shield he bears.
        shield: bool,
    },
}

pub fn projectile_birth_ms(projectile: &ProjectileState) -> i32 {
    projectile.spawn_time_ms
}

fn forward(angles: [f32; 3]) -> [f32; 3] {
    let pitch = angles[0].to_radians();
    let yaw = angles[1].to_radians();
    [
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        -pitch.sin(),
    ]
}

fn vec3_length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn vec3_normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = vec3_length(v);
    if len < 1e-6 {
        None
    } else {
        Some([v[0] / len, v[1] / len, v[2] / len])
    }
}

fn flatten_xy(dir: [f32; 3]) -> Option<[f32; 3]> {
    vec3_normalize([dir[0], dir[1], 0.0])
}

fn project_owner_velocity(launch_vel: [f32; 3], owner_vel: [f32; 3]) -> [f32; 3] {
    let Some(dir) = vec3_normalize(launch_vel) else {
        return launch_vel;
    };
    let along = owner_vel[0] * dir[0] + owner_vel[1] * dir[1] + owner_vel[2] * dir[2];
    [
        launch_vel[0] + dir[0] * along,
        launch_vel[1] + dir[1] * along,
        launch_vel[2] + dir[2] * along,
    ]
}

fn grenade_launch_velocity(
    angles: [f32; 3],
    facts: &EquipmentRuntimeFacts,
    owner_vel: [f32; 3],
) -> [f32; 3] {
    let direction = forward(angles);
    let speed = facts.projectile_speed as f32;
    let mut velocity = [
        direction[0] * speed,
        direction[1] * speed,
        direction[2] * speed + facts.projectile_speed_up as f32,
    ];
    if facts.projectile_speed_forward != 0
        && let Some(flat) = flatten_xy(direction)
    {
        let extra = facts.projectile_speed_forward as f32;
        velocity[0] += flat[0] * extra;
        velocity[1] += flat[1] * extra;
        velocity[2] += flat[2] * extra;
    }
    // bo2zm: a Black Ops II throw also rises along the view's up axis (the
    // frag: 920 forward, 120 up).
    if facts.projectile_speed_relative_up != 0 {
        let (_, _, up) = math_iw4::angle_vectors(angles);
        let extra = facts.projectile_speed_relative_up as f32;
        velocity[0] += up[0] * extra;
        velocity[1] += up[1] * extra;
        velocity[2] += up[2] * extra;
    }
    // bo2mp window 5: Black Ops II's server adds the thrower's velocity along
    // the throw, the same rule as Modern Warfare 2; its weapon files' parent
    // velocity share is only read for vehicles. Taking that share (the frag's
    // is none) dropped a running throw short.
    project_owner_velocity(velocity, owner_vel)
}

fn grenade_fuse_due(now_ms: i32, detonate_at_ms: Option<i32>, live: bool) -> bool {
    detonate_at_ms.is_some_and(|deadline| live && deadline <= now_ms)
}

// bo2mp window 5: the players a grenade can meet. Black Ops 2 meets them only on the server,
// by their hit boxes as a bullet does; its client draws the server's path and never bounces a
// grenade off a player. A prediction copy knows the other players only as a whole-body box, so
// its grenades pass them by and the next snapshot brings the bounce (no pop up off the box top).
fn missile_bodies(world: &FrameWorld) -> Vec<crate::bullet_collision::PlayerCollisionPose> {
    let mut players = world.alive_collision_poses();
    let remote = world.prediction_remote_bodies();
    players.retain(|pose| !remote.iter().any(|body| body.client == pose.client));
    players
}

pub(crate) const AIRDROP_MARKER_WEAPON: &str = "airdrop_marker_mp";

// The airdrop marker's fuse would otherwise pop the flare midair on a steep throw. Black Ops 2
// flags its own (concussion, EMP, smoke): a fuse that ends in the air is retried every frame
// until the grenade is on a floor.
fn waits_for_ground(world: &FrameWorld, facts: &EquipmentRuntimeFacts, weapon: u32) -> bool {
    facts.explode_on_ground
        || facts.offhand_class == OFFHAND_CLASS_SMOKE
        || world.weapon_script_name(weapon) == AIRDROP_MARKER_WEAPON
}

fn grenade_deadlines(
    facts: &EquipmentRuntimeFacts,
    kind: GrenadeLaunchKind,
    now_ms: i32,
) -> (Option<i32>, i32) {
    let cap_at = now_ms.saturating_add(GRENADE_FUSE_CAP_MS);
    if facts.is_throwing_knife() {
        return (None, cap_at);
    }
    let cleanup_at_ms = cap_at;
    match kind {
        GrenadeLaunchKind::InHand => (Some(now_ms.saturating_add(1)), cleanup_at_ms),
        // A fuse past the cap (the buildables' 30000 s) never goes off.
        GrenadeLaunchKind::Magic { fuse_ms } if fuse_ms >= GRENADE_FUSE_CAP_MS => {
            (None, cleanup_at_ms)
        }
        GrenadeLaunchKind::Magic { fuse_ms } => {
            (Some(now_ms.saturating_add(fuse_ms.max(1))), cleanup_at_ms)
        }
        GrenadeLaunchKind::Launcher if facts.fuse_time_ms <= 0 => {
            if facts.projectile_activate_dist != 0 {
                (None, cleanup_at_ms)
            } else {
                (
                    Some(now_ms.saturating_add(GRENADE_DEFAULT_FUSE_MS).min(cap_at)),
                    cleanup_at_ms,
                )
            }
        }
        GrenadeLaunchKind::Thrown { remaining_fuse_ms } => {
            let authored = facts.fuse_time_ms;
            let fuse = if facts.timed_detonation {
                remaining_fuse_ms.filter(|&ms| ms > 0).unwrap_or(authored)
            } else {
                authored
            };
            let fuse = if fuse <= 0 {
                GRENADE_DEFAULT_FUSE_MS
            } else {
                fuse.min(GRENADE_FUSE_CAP_MS)
            };
            (Some(now_ms.saturating_add(fuse)), cleanup_at_ms)
        }
        // BO2 times a dropped grenade from the fuse it hands over, not the weapon's own.
        GrenadeLaunchKind::Dropped { remaining_fuse_ms, .. } if facts.timed_detonation => (
            Some(now_ms.saturating_add(remaining_fuse_ms.min(GRENADE_FUSE_CAP_MS))),
            cleanup_at_ms,
        ),
        GrenadeLaunchKind::Dropped { .. } => {
            grenade_deadlines(facts, GrenadeLaunchKind::Thrown { remaining_fuse_ms: None }, now_ms)
        }
        GrenadeLaunchKind::Launcher => {
            let fuse = if facts.fuse_time_ms <= 0 {
                GRENADE_DEFAULT_FUSE_MS
            } else {
                facts.fuse_time_ms.min(GRENADE_FUSE_CAP_MS)
            };
            (Some(now_ms.saturating_add(fuse)), cleanup_at_ms)
        }
    }
}

pub(crate) fn spawn_grenade_projectile(
    world: &mut FrameWorld,
    owner: ClientId,
    weapon: u32,
    tick: Tick,
    origin: [f32; 3],
    angles: [f32; 3],
    owner_vel: [f32; 3],
    kind: GrenadeLaunchKind,
) -> bool {
    let Some(facts) = world.equipment_facts_for(weapon) else {
        // bo2zm: say why a thrown or launched weapon spawned nothing.
        let raw = world.equipment_row_raw(weapon);
        diag::warn!(
            Sim,
            "grenade not spawned: weapon {weapon} has no usable equipment row ({})",
            raw.map_or("none".to_owned(), |f| format!(
                "speed {} sticky {} fuse {}ms impact {} inner {}",
                f.projectile_speed, f.stickiness, f.fuse_time_ms, f.impact_damage, f.explosion_inner_damage
            ))
        );
        return false;
    };
    if !facts.is_usable() {
        return false;
    }
    let owner_life = world.client_meta(owner).unwrap().life_sequence;
    let velocity = match kind {
        GrenadeLaunchKind::Dropped { velocity, .. } => velocity,
        GrenadeLaunchKind::InHand | GrenadeLaunchKind::Magic { .. } => owner_vel,
        _ => grenade_launch_velocity(angles, &facts, owner_vel),
    };
    let direction = match kind {
        GrenadeLaunchKind::Dropped { .. } => vec3_normalize(velocity).unwrap_or([0.0, 0.0, 1.0]),
        _ => forward(angles),
    };
    let time_ms = level_time_ms(tick);
    let (pitch_rate, roll_rate) = match kind {
        GrenadeLaunchKind::Launcher
        | GrenadeLaunchKind::InHand
        | GrenadeLaunchKind::Magic { .. } => (0.0, 0.0),
        _ if facts.weap_class == WEAPCLASS_THROWINGKNIFE => {
            (entity_iw4::GRENADE_BLADE_SPIN_PITCH, 0.0)
        }
        _ if facts.projectile_rotates => grenade_spin_rates(world),
        _ => (0.0, 0.0),
    };
    let apos = if pitch_rate == 0.0 && roll_rate == 0.0 {
        fire_missile_apos(direction)
    } else {
        init_grenade_apos(direction, time_ms, pitch_rate, roll_rate)
    };
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "throw: weapon {weapon} from {:?} angles [{:.1}, {:.1}] velocity {:?} owner {:?}",
            origin.map(f32::round),
            angles[0],
            angles[1],
            velocity.map(f32::round),
            owner_vel.map(f32::round)
        );
    }
    let pos = init_grenade_pos(origin, velocity, time_ms);
    let speed = vec3_length(velocity);
    let launch_time = time_ms + fire_grenade_no_draw_ms(speed);
    let (detonate_at_ms, cleanup_at_ms) = grenade_deadlines(&facts, kind, time_ms);
    let id_projectile = world.allocate_projectile_id();
    let entnum = match world.allocate_dynamic_entity(crate::gentity::EntityRunKind::Missile) {
        Ok(entity) => entity.number(),
        Err(error) => {
            diag::warn!(Sim, "grenade not spawned: {error:?}");
            return false;
        }
    };
    world.push_projectile(ProjectileState {
        id: id_projectile,
        owner,
        owner_life,
        weapon,
        origin,
        velocity: pos.tr_delta,
        pos,
        apos,
        entnum,
        launch_time,
        spawn_time_ms: time_ms,
        detonate_at_ms,
        cleanup_at_ms,
        travel_distance: 0.0,
        live: true,
        stuck_pane: None,
        grounded: false,
        guide: crate::MissileGuide::default(),
        impact_target: None,
        rider: None,
    });
    true
}

pub(crate) fn spawn_offhand_projectile(
    world: &mut FrameWorld,
    id: ClientId,
    weapon: u32,
    tick: Tick,
    remaining_fuse_ms: Option<i32>,
) -> bool {
    let (origin, angles, owner_vel) = {
        let Some(ps) = world.player(id) else {
            return false;
        };
        (
            [
                ps.origin[0],
                ps.origin[1],
                ps.origin[2] + ps.view_height_current,
            ],
            ps.viewangles,
            ps.velocity,
        )
    };
    spawn_grenade_projectile(
        world,
        id,
        weapon,
        tick,
        origin,
        angles,
        owner_vel,
        GrenadeLaunchKind::Thrown { remaining_fuse_ms },
    )
}

/// bo2zm: a grenade cooked too long goes off in his hand as a real
/// grenade (its explosion effect and sound, its damage through the game's
/// rules); false when none could spawn.
pub(crate) fn spawn_offhand_in_hand(world: &mut FrameWorld, id: ClientId, weapon: u32, tick: Tick) -> bool {
    let Some(ps) = world.player(id).copied() else {
        return false;
    };
    let origin = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current - 8.0];
    spawn_grenade_projectile(world, id, weapon, tick, origin, ps.viewangles, ps.velocity, GrenadeLaunchKind::InHand)
}

/// BO2's death grenade drop: a player who dies holding a live grenade lets it go where they
/// stand (40 up unless it sticks to anything), tossed up to 160 a second each way, its fuse
/// still running. A cook-off's spent fuse makes it go off at once: the blast, its sound and
/// its damage all come from the grenade, credited to them.
pub(crate) fn drop_grenade_on_death(
    world: &mut FrameWorld,
    id: ClientId,
    weapon: u32,
    tick: Tick,
    remaining_fuse_ms: i32,
) -> bool {
    let Some(facts) = world.equipment_facts_for(weapon) else {
        return false;
    };
    let Some(ps) = world.player(id).copied() else {
        return false;
    };
    let mut origin = ps.origin;
    if facts.stickiness != 1 {
        origin[2] += 40.0;
    }
    let rng = world.combat_rng_mut();
    let velocity: [f32; 3] = core::array::from_fn(|_| host_flrand(rng, -160.0, 160.0));
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "death drop: weapon {weapon} fuse {remaining_fuse_ms}ms from {:?} velocity {:?}",
            origin.map(f32::round),
            velocity.map(f32::round)
        );
    }
    spawn_grenade_projectile(
        world,
        id,
        weapon,
        tick,
        origin,
        ps.viewangles,
        [0.0; 3],
        GrenadeLaunchKind::Dropped { remaining_fuse_ms, velocity },
    )
}

pub(crate) fn explode_offhand_in_hand(
    world: &mut FrameWorld,
    id: ClientId,
    weapon: u32,
    tick: Tick,
) {
    let Some(facts) = world.equipment_facts_for(weapon) else {
        return;
    };
    let Some(ps) = world.player(id).copied() else {
        return;
    };
    let owner_life = world
        .client_meta(id)
        .map(|m| m.life_sequence)
        .unwrap_or_default();
    let origin = [
        ps.origin[0],
        ps.origin[1],
        ps.origin[2] + ps.view_height_current,
    ];
    let radius = facts
        .explosion_radius
        .max(facts.explosion_radius_min)
        .max(0) as f32;
    if facts.projectile_explosion_type == 2 {
        crate::damage::apply_flashbang_blast(
            world,
            origin,
            facts.explosion_radius.max(0) as f32,
            facts.explosion_radius_min.max(0) as f32,
            id,
        );
    }
    if radius <= 0.0 || facts.explosion_inner_damage <= 0 {
        return;
    }
    let blast = crate::damage::ExplosionBlast {
        cone: None,
        origin,
        radius,
        inner_damage: facts.explosion_inner_damage as f32,
        outer_damage: facts.explosion_outer_damage.max(0) as f32,
        weapon,
        source: DamageSource::Projectile(ProjectileId(0)),
        attacker: id,
        attacker_life: owner_life,
        killcam_entity_start_time: level_time_ms(tick),
    };
    crate::damage::apply_explosion_blast(world, tick, &blast);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectileHitGeometry {
    World,
    Player,
    Entity { epoch: EntityCollisionEpoch },
    Bounce,
    Fuse,
    Dud,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileImpact {
    pub id: ProjectileId,
    pub owner: ClientId,
    pub weapon: u32,
    pub origin: [f32; 3],
    pub geometry: ProjectileHitGeometry,
    pub terminal: Option<ColliderId>,
    pub amount: i32,
    pub fraction: Option<f32>,
    pub surface_flags: Option<u32>,
    pub surf_type: Option<u8>,
    pub entnum: i32,
}

fn grenade_spin_rates(world: &mut FrameWorld) -> (f32, f32) {
    let rng = world.combat_rng_mut();
    let pitch_sign = if rng.next_index(2) == 0 { 1.0 } else { -1.0 };
    let pitch = host_flrand(rng, GRENADE_SPIN_PITCH_MIN, GRENADE_SPIN_PITCH_MAX);
    let roll_sign = if rng.next_index(2) == 0 { 1.0 } else { -1.0 };
    let roll = host_flrand(rng, GRENADE_SPIN_ROLL_MIN, GRENADE_SPIN_ROLL_MAX);
    (pitch_sign * pitch, roll_sign * roll)
}

fn host_flrand(rng: &mut crate::MatchRng, min: f32, max: f32) -> f32 {
    let u = rng.next_u32() as f32 * (1.0 / 4_294_967_296.0);
    min + (max - min) * u
}

pub(crate) fn predict_projectile(
    world: &FrameWorld,
    mut projectile: ProjectileState,
    now: i32,
    duration_ms: i32,
) -> Option<[f32; 3]> {
    let facts = required_projectile_facts(world, projectile.weapon);
    let attached = world.missile_collision_models(projectile.id);
    let models: Vec<_> = world
        .entity_collision_capabilities()
        .iter()
        .filter(|capabilities| {
            !capabilities
                .owner
                .script_model()
                .is_some_and(|id| attached.contains(&id))
        })
        .map(|capabilities| capabilities.trace_geom())
        .collect();
    let content = world.content();
    let (brushes, bsp, cmodels, mesh) = (
        content.clip_brushes(),
        content.clip_bsp(),
        content.clip_cmodels(),
        content.clip_mesh(),
    );
    let players = missile_bodies(world);
    let deadline = now
        .saturating_add(duration_ms)
        .min(projectile.cleanup_at_ms)
        .min(projectile.detonate_at_ms.unwrap_or(i32::MAX));
    let mut time = now;
    while time < deadline {
        if projectile.pos.tr_type == TR_STATIONARY {
            return Some(projectile.origin);
        }
        let next = time
            .saturating_add(crate::MATCH_TICK_MS as i32)
            .min(deadline);
        crate::missile_guidance::steer(world, &mut projectile, next, facts);
        let end = projectile.origin_at(next);
        match bullet_trace_with_entity_models(
            &brushes,
            &bsp,
            &cmodels,
            &mesh,
            &players,
            &models,
            &BulletTraceQuery {
                start: projectile.origin,
                end,
                mask: MASK_BULLET_WORLD,
                ignore: Some(projectile.owner),
                ignore_hit: None,
                ignore_model: None,
            },
            &|piece| world.world_objects().glass_is_solid(u32::from(piece)),
        ) {
            TraceOutcome::Hit {
                end,
                normal,
                collider,
                fraction,
            } => {
                projectile.travel_distance +=
                    vec3_length(core::array::from_fn(|i| end[i] - projectile.origin[i]));
                if (matches!(collider, ColliderId::Player { .. }) && facts.stick_to_players)
                    || (!matches!(collider, ColliderId::Player { .. })
                        && sticks_to_surface(facts, normal))
                    || (facts.proj_impact_explode
                        && projectile.is_armed(facts.projectile_activate_dist))
                {
                    return Some(end);
                }
                let surf_type = match collider {
                    ColliderId::World { surface_flags, .. }
                    | ColliderId::EntityDObjBone { surface_flags, .. }
                    | ColliderId::EntityLinkedBrush { surface_flags, .. } => {
                        surface_type_from_flags(surface_flags)
                    }
                    ColliderId::Player { .. } => 0,
                };
                let hit_time = time + ((next - time) as f32 * fraction) as i32;
                projectile.velocity = projectile.velocity_at(hit_time);
                bounce_velocity(&mut projectile, normal, &facts, surf_type);
                projectile.origin = end;
                projectile.pos.tr_base = end;
                projectile.pos.tr_time = next;
                projectile.pos.tr_delta = projectile.velocity;
            }
            TraceOutcome::Miss { .. } => {
                projectile.travel_distance +=
                    vec3_length(core::array::from_fn(|i| end[i] - projectile.origin[i]));
                projectile.origin = end;
            }
            TraceOutcome::StartSolid { .. } | TraceOutcome::Invalid { .. } => return None,
        }
        time = next;
    }
    Some(projectile.origin)
}

pub(crate) fn think_projectile(world: &mut FrameWorld, tick: Tick, entnum: i32) {
    struct PendingDetonation {
        projectile: ProjectileState,
        origin: [f32; 3],
        normal: [f32; 3],
        surf_type: u8,
        geometry: ProjectileHitGeometry,
        terminal: Option<ColliderId>,
        amount: i32,
        fraction: Option<f32>,
        surface_flags: Option<u32>,
        splash: bool,
    }
    let mut detonated = Vec::new();
    let mut direct_hits = Vec::new();
    let mut entity_hits = Vec::new();
    let mut impacts = Vec::new();
    let Some(mut projectile) = world.projectile_by_number(entnum) else {
        return;
    };
    let time = level_time_ms(tick);
    let prev = time.saturating_sub(crate::MATCH_TICK_MS as i32);
    if time >= projectile.cleanup_at_ms
        && projectile
            .detonate_at_ms
            .is_none_or(|deadline| deadline > projectile.cleanup_at_ms || !projectile.live)
    {
        let _ = world.remove_projectile_by_number(entnum);
        return;
    }
    let facts = required_projectile_facts(world, projectile.weapon);
    crate::missile_guidance::steer(world, &mut projectile, time, facts);
    let attached = world.missile_collision_models(projectile.id);
    let script_models: Vec<EntityCollisionTraceGeom> = world
        .entity_collision_capabilities()
        .iter()
        .filter(|capabilities| {
            !capabilities
                .owner
                .script_model()
                .is_some_and(|id| attached.contains(&id))
        })
        .map(|capabilities| capabilities.trace_geom())
        .collect();
    let content = world.content();
    let cmodels = content.clip_cmodels();
    let players = missile_bodies(world);
    let (brushes, bsp, mesh) = (
        content.clip_brushes(),
        content.clip_bsp(),
        content.clip_mesh(),
    );
    // bo2mp: a missile stuck to a player rides on the box it hit while he
    // lives (Black Ops II links it to him; on his death it stays put here).
    if let Some(rider) = projectile.rider {
        match rider_origin(&players, rider) {
            Some(at) => {
                projectile.origin = at;
                projectile.pos.tr_base = at;
            }
            None => projectile.rider = None,
        }
        if let Some(row) = world.projectile_mut_by_number(entnum) {
            *row = projectile;
        }
    }
    let start = projectile.origin;
    let mut eval_time = time;
    if let Some(deadline) = projectile
        .detonate_at_ms
        .filter(|&ms| projectile.live && ms <= time)
    {
        eval_time = eval_time.min(deadline);
    }
    if projectile.cleanup_at_ms > prev && projectile.cleanup_at_ms <= eval_time {
        eval_time = eval_time.min(projectile.cleanup_at_ms);
    }
    let end = projectile.origin_at(eval_time);
    let vel = projectile.velocity_at(eval_time);
    let speed = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
    if let Some(pane) = projectile.stuck_pane {
        if world.world_objects().glass_is_solid(pane)
            && projectile
                .detonate_at_ms
                .is_none_or(|deadline| deadline > time)
        {
            if let Some(row) = world.projectile_mut_by_number(entnum) {
                *row = projectile;
            }
            return;
        }
        if !world.world_objects().glass_is_solid(pane) {
            unstick_missile(tick, &mut projectile);
            projectile.stuck_pane = None;
        }
    }
    if projectile.pos.tr_type == TR_STATIONARY
        && (facts.is_retrievable_knife() || projectile.detonate_at_ms.is_none())
    {
        return;
    }
    // Test log (IW4L_SHOT_LOG): the bodies this stretch could strike, and the
    // nearest hit box to it (a bolt that flies through a zombie).
    if crate::combat::shot_log() {
        let along = |p: [f32; 3]| {
            let d: [f32; 3] = core::array::from_fn(|i| end[i] - start[i]);
            let len2 = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).max(1e-6);
            let t = (((p[0] - start[0]) * d[0] + (p[1] - start[1]) * d[1] + (p[2] - start[2]) * d[2])
                / len2)
                .clamp(0.0, 1.0);
            vec3_length(core::array::from_fn(|i| p[i] - (start[i] + d[i] * t)))
        };
        let near = script_models
            .iter()
            .filter_map(|g| Some((g.owner, g.collision.as_ref()?)))
            .flat_map(|(o, c)| c.bones.iter().map(move |b| (o, b.bone, b.center)))
            .map(|(o, bone, c)| (o, bone, c.map(f32::round), along(c).round()))
            .min_by(|a, b| a.3.total_cmp(&b.3));
        diag::info!(
            Sim,
            "nade tick {time}: {} of {} models posed, nearest box {near:?}",
            script_models.iter().filter(|g| g.collision.is_some()).count(),
            script_models.len()
        );
        // The zombie nearest the path, whatever its hit boxes: why it has
        // none (not solid, never posed, or posed then dropped).
        let nearest = world
            .entity_collision_capabilities()
            .iter()
            .filter_map(|row| {
                let d = row.dobj.as_ref().filter(|d| d.current_model.contains("zom"))?;
                let at = d.world_from_model.w_axis.truncate().to_array();
                Some((along(at), row, d))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((dist, row, d)) = nearest {
            diag::info!(
                Sim,
                "nade tick {time}: nearest model {:?} {} {dist:.0} away: solid {} hidden {} boxes {} model rev {}/{:?} pose rev {}/{:?}",
                row.owner,
                d.current_model,
                row.solid,
                row.hidden,
                d.current_collision.is_some(),
                d.model_revision,
                d.materialized_model_revision,
                d.pose_revision,
                d.materialized_pose_revision
            );
        }
    }
    let mut trace_start = start;
    let mut hops = 0u32;
    let mut hop_capped = false;
    let outcome = loop {
        let glass_pairs = world.world_objects().glass_damage_pairs();
        let outcome = missile_trace(
            &players,
            &BulletTraceQuery {
                start: trace_start,
                end,
                mask: MASK_BULLET_WORLD,
                ignore: Some(projectile.owner),
                ignore_hit: None,
                ignore_model: None,
            },
            &|players, query| {
                bullet_trace_with_entity_models(
                    &brushes,
                    &bsp,
                    &cmodels,
                    &mesh,
                    players,
                    &script_models,
                    query,
                    &|piece| {
                        crate::world_objects::glass_piece_is_solid(
                            glass_pairs
                                .iter()
                                .find(|(id, _)| *id == u32::from(piece))
                                .map(|(_, d)| *d)
                                .unwrap_or(0),
                        )
                    },
                )
            },
        );
        let punched = match outcome {
            TraceOutcome::Hit {
                end: hit_end,
                collider:
                    ColliderId::World {
                        surface_flags,
                        glass_encoded,
                        ..
                    },
                ..
            }
            | TraceOutcome::StartSolid {
                end: hit_end,
                collider:
                    Some(ColliderId::World {
                        surface_flags,
                        glass_encoded,
                        ..
                    }),
            } if missile_glass_punch_eligible(speed, surface_flags, glass_encoded) => {
                let pane = u32::from(glass_encoded).saturating_sub(1);
                if world.world_objects().glass_is_solid(pane) {
                    let _ = world.world_objects_mut().force_shatter_glass(
                        pane,
                        time,
                        hit_end,
                        vel,
                        entity_iw4::GlassCause::Impact,
                    );
                    hops = hops.saturating_add(1);
                    if hops >= GLASS_PROJECTILE_PANE_HOPS {
                        hop_capped = true;
                        park_missile_at(tick, &mut projectile, hit_end, vel);
                        None
                    } else {
                        let dx = end[0] - trace_start[0];
                        let dy = end[1] - trace_start[1];
                        let dz = end[2] - trace_start[2];
                        let len = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);
                        trace_start = [
                            hit_end[0] + dx / len * 0.25,
                            hit_end[1] + dy / len * 0.25,
                            hit_end[2] + dz / len * 0.25,
                        ];
                        Some(())
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if punched.is_some() && !hop_capped {
            continue;
        }
        break outcome;
    };
    if hop_capped {
        projectile.velocity = vel;
        if let Some(row) = world.projectile_mut_by_number(entnum) {
            *row = projectile;
        }
        return;
    }
    let mut pending_detonation = None;
    let mut hit = match outcome {
        TraceOutcome::Hit {
            fraction,
            end,
            normal,
            collider,
        } => Some((end, normal, Some(collider), fraction)),
        // bo2mp window 5: Black Ops II's missile trace that starts inside something faces back
        // along the move (from where it was asked to go to where it began).
        TraceOutcome::StartSolid {
            collider,
            end: solid_end,
        } => Some((solid_end, startsolid_normal(trace_start, end), collider, 0.0)),
        TraceOutcome::Miss { .. } | TraceOutcome::Invalid { .. } => None,
    };
    // Riding on him, it touches nothing more.
    if projectile.rider.is_some() {
        hit = None;
    }
    let mut contact_origin = None;
    if facts.is_retrievable_knife()
        && let Some((end, normal, collider, _)) = hit
        && !matches!(collider, Some(ColliderId::Player { .. }))
    {
        let probe_start = core::array::from_fn(|i| end[i] + normal[i] * 0.135);
        let probe_end = core::array::from_fn(|i| end[i] - normal[i] * 1.5);
        if let TraceOutcome::Hit {
            end,
            normal,
            collider,
            fraction,
        } = bullet_trace_with_entity_models(
            &brushes,
            &bsp,
            &cmodels,
            &mesh,
            &players,
            &script_models,
            &BulletTraceQuery {
                start: probe_start,
                end: probe_end,
                mask: MASK_BULLET_WORLD,
                ignore: Some(projectile.owner),
                ignore_hit: None,
                ignore_model: None,
            },
            &|piece| world.world_objects().glass_is_solid(u32::from(piece)),
        ) {
            // Surface clearance precedes the knife's resting/embedded pose offsets.
            contact_origin = Some(core::array::from_fn(|i| end[i] + (end[i] - probe_end[i])));
            hit = Some((end, normal, Some(collider), fraction));
        }
    }
    let mut contact_time = eval_time;
    if let Some((_end, _normal, _collider, fraction)) = hit {
        contact_time = prev.saturating_add(((eval_time - prev) as f32 * fraction) as i32);
    }
    // bo2mp window 5: how fast it was going when it touched (not at the
    // tick's end): Black Ops II judges a soft or hard hit on that speed.
    let contact_vel = projectile.velocity_at(contact_time);
    let mut fuse_due = grenade_fuse_due(time, projectile.detonate_at_ms, projectile.live);
    if fuse_due && !projectile.grounded && waits_for_ground(world, &facts, projectile.weapon) {
        fuse_due = false;
        projectile.detonate_at_ms = Some(time.saturating_add(crate::MATCH_TICK_MS as i32));
    }
    let cleanup_due = time >= projectile.cleanup_at_ms;
    let contact_before_fuse = hit.is_some()
        && !(projectile.pos.tr_type == TR_STATIONARY && fuse_due)
        && projectile
            .detonate_at_ms
            .is_none_or(|deadline| contact_time <= deadline);
    enum TickOutcome {
        Contact,
        Fuse,
        Cleanup,
        Fly,
    }
    let tick_outcome = if hit.is_some() && contact_before_fuse {
        TickOutcome::Contact
    } else if fuse_due {
        TickOutcome::Fuse
    } else if cleanup_due {
        TickOutcome::Cleanup
    } else {
        TickOutcome::Fly
    };
    // bo2mp window 5: how far a bounced grenade gets on a short move off the surface it hit
    // (the same missile trace, the thrower ignored).
    let step_glass = world.world_objects().glass_damage_pairs();
    let step_owner = projectile.owner;
    let step_off = |from: [f32; 3], to: [f32; 3]| -> f32 {
        match missile_trace(
            &players,
            &BulletTraceQuery {
                start: from,
                end: to,
                mask: MASK_BULLET_WORLD,
                ignore: Some(step_owner),
                ignore_hit: None,
                ignore_model: None,
            },
            &|players, query| {
                bullet_trace_with_entity_models(
                    &brushes,
                    &bsp,
                    &cmodels,
                    &mesh,
                    players,
                    &script_models,
                    query,
                    &|piece| {
                        crate::world_objects::glass_piece_is_solid(
                            step_glass
                                .iter()
                                .find(|(id, _)| *id == u32::from(piece))
                                .map(|(_, d)| *d)
                                .unwrap_or(0),
                        )
                    },
                )
            },
        ) {
            TraceOutcome::Hit { fraction, .. } => fraction,
            TraceOutcome::Miss { .. } => 1.0,
            TraceOutcome::StartSolid { .. } | TraceOutcome::Invalid { .. } => 0.0,
        }
    };
    // bo2zm fix list 2: the surface a rolling grenade touched this tick.
    let mut contact_normal: Option<[f32; 3]> = None;
    match tick_outcome {
        TickOutcome::Cleanup => {
            let _ = world.remove_projectile_by_number(entnum);
            return;
        }
        TickOutcome::Fuse => {
            let fuse_time = projectile.detonate_at_ms.unwrap_or(eval_time);
            projectile.origin = projectile.origin_at(fuse_time);
            projectile.velocity = projectile.velocity_at(fuse_time);
            pending_detonation = Some(PendingDetonation {
                projectile,
                origin: projectile.origin,
                normal: [0.0, 0.0, 1.0],
                surf_type: 0,
                geometry: ProjectileHitGeometry::Fuse,
                terminal: None,
                amount: 0,
                fraction: None,
                surface_flags: None,
                splash: projectile.live,
            });
        }
        TickOutcome::Fly => {
            if crate::combat::shot_log() {
                diag::info!(
                    Sim,
                    "nade tick {time}: fly to {:?} vel {:?}",
                    end.map(|v| (v * 10.0).round() / 10.0),
                    vel.map(f32::round)
                );
            }
            let traveled = vec3_length([end[0] - start[0], end[1] - start[1], end[2] - start[2]]);
            // A frame in free flight leaves the ground, as Black Ops 2's: a grenade that hopped
            // off a floor waits for its next floor before a ground-only fuse goes off.
            if speed != 0.0 {
                projectile.grounded = false;
            }
            projectile.origin = end;
            projectile.travel_distance = projectile.travel_distance + traveled;
            projectile.velocity = projectile.velocity_at(time);
            // bo2zm fix list 2: a rolling grenade on the ground goes on to
            // its roll below (ground, slope, resistance) every tick.
            let rolling = facts.rolling_radius.is_some()
                && projectile.pos.tr_type == entity_iw4::TR_LINEAR;
            if !rolling {
                if let Some(row) = world.projectile_mut_by_number(entnum) {
                    *row = projectile;
                }
                return;
            }
        }
        TickOutcome::Contact => {
            let (end, normal, collider, fraction) = hit.expect("contact");
            contact_normal = Some(normal);
            if crate::combat::shot_log() {
                diag::info!(
                    Sim,
                    "nade tick {time}: contact at {:?} normal {:?} frac {fraction:.2} vel {:?} collider {:?}",
                    end.map(|v| (v * 10.0).round() / 10.0),
                    normal.map(|v| (v * 100.0).round() / 100.0),
                    vel.map(f32::round),
                    collider
                );
            }
            let traveled = vec3_length([end[0] - start[0], end[1] - start[1], end[2] - start[2]]);
            projectile.origin = contact_origin.unwrap_or(end);
            projectile.travel_distance = projectile.travel_distance + traveled;
            let armed = projectile.is_armed(facts.projectile_activate_dist);
            let (surface_flags, hit_surf) = match collider {
                Some(ColliderId::World { surface_flags, .. }) => (
                    Some(surface_flags),
                    Some(surface_type_from_flags(surface_flags)),
                ),
                _ => (None, None),
            };
            match collider {
                _ if facts.is_retrievable_knife() => {
                    let player_hit = matches!(collider, Some(ColliderId::Player { .. }));
                    if let Some(ColliderId::Player { client, .. }) = collider {
                        direct_hits.push((projectile, client));
                    }
                    knife_impact(
                        world,
                        tick,
                        &mut projectile,
                        normal,
                        fraction,
                        hit_surf.unwrap_or(0),
                        player_hit,
                    );
                    if projectile.pos.tr_type == TR_STATIONARY
                        && let Some(ColliderId::World { glass_encoded, .. }) = collider
                        && glass_encoded != 0
                    {
                        projectile.stuck_pane = Some(u32::from(glass_encoded) - 1);
                    }
                    impacts.push(ProjectileImpact {
                        id: projectile.id,
                        owner: projectile.owner,
                        weapon: projectile.weapon,
                        origin: end,
                        geometry: if player_hit {
                            ProjectileHitGeometry::Player
                        } else {
                            ProjectileHitGeometry::Bounce
                        },
                        terminal: collider,
                        amount: if player_hit {
                            facts.impact_damage.max(0)
                        } else {
                            0
                        },
                        fraction: Some(fraction),
                        surface_flags,
                        surf_type: hit_surf,
                        entnum: projectile.entnum,
                    });
                }
                Some(ColliderId::Player {
                    client,
                    life,
                    hitloc,
                }) => {
                    if crate::combat::shot_log() {
                        diag::info!(
                            Sim,
                            "nade tick {time}: player {client:?} hitloc {hitloc} stick {} armed {armed} impact explode {}",
                            facts.stick_to_players,
                            facts.proj_impact_explode
                        );
                    }
                    let impact = !facts.limit_grenade_impacts
                        || projectile.impact_target.replace(client) != Some(client);
                    if facts.stick_to_players {
                        // Black Ops II hits, then sticks.
                        if facts.limit_grenade_impacts && impact {
                            direct_hits.push((projectile, client));
                        }
                        stick_missile(tick, &mut projectile, end);
                        let shield = hitloc == crate::bullet_collision::HITLOC_SHIELD;
                        projectile.rider = ride_on(&players, client, life, hitloc, end)
                            .map(|r| crate::equipment::StuckRider { shield, ..r });
                        push_grenade_stick(world, tick, &projectile, Some(client), shield);
                    } else if !armed {
                        direct_hits.push((projectile, client));
                        projectile.live = false;
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type: 0,
                            geometry: ProjectileHitGeometry::Dud,
                            terminal: collider,
                            amount: facts.impact_damage.max(0),
                            fraction: Some(fraction),
                            surface_flags: None,
                            splash: false,
                        });
                    } else if facts.proj_impact_explode || facts.rocket_ends_on_impact() {
                        direct_hits.push((projectile, client));
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type: 0,
                            geometry: ProjectileHitGeometry::Player,
                            terminal: collider,
                            amount: facts.impact_damage.max(0),
                            fraction: Some(fraction),
                            surface_flags: None,
                            splash: true,
                        });
                    } else {
                        if impact {
                            direct_hits.push((projectile, client));
                        }
                        bounce_missile(world, tick, &mut projectile, end, normal, fraction, 0, &step_off);
                        impacts.push(ProjectileImpact {
                            id: projectile.id,
                            owner: projectile.owner,
                            weapon: projectile.weapon,
                            origin: end,
                            geometry: ProjectileHitGeometry::Bounce,
                            terminal: collider,
                            amount: 0,
                            fraction: Some(fraction),
                            surface_flags: None,
                            surf_type: None,
                            entnum: projectile.entnum,
                        });
                    }
                }
                Some(
                    collider @ (ColliderId::EntityDObjBone { .. }
                    | ColliderId::EntityLinkedBrush { .. }),
                ) => {
                    let epoch = entity_collision_epoch(collider, &script_models)
                        .unwrap_or(EntityCollisionEpoch::CurrentTick);
                    if armed && facts.proj_impact_explode {
                        if let Some(target) = match collider {
                            ColliderId::EntityDObjBone { owner, .. }
                            | ColliderId::EntityLinkedBrush { owner, .. } => owner.script_model(),
                            _ => None,
                        } {
                            let bone = match collider {
                                ColliderId::EntityDObjBone { bone, .. } => Some(usize::from(bone)),
                                _ => None,
                            };
                            entity_hits.push((
                                DamageSource::Projectile(projectile.id),
                                crate::script::EntityHit {
                                    target,
                                    amount: facts.impact_damage.max(0),
                                    attacker: Some(projectile.owner),
                                    means: "",
                                    weapon: projectile.weapon,
                                    point: end,
                                    dir: projectile.velocity,
                                    bone,
                                    flags: 0,
                                },
                            ));
                        }
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type: 0,
                            geometry: ProjectileHitGeometry::Entity { epoch },
                            terminal: Some(collider),
                            amount: facts.impact_damage.max(0),
                            fraction: Some(fraction),
                            surface_flags: None,
                            splash: true,
                        });
                    } else {
                        // bo2zm: a missile that does not explode still hits the
                        // entity it strikes for its impact damage (MOD_IMPACT),
                        // as players are hit above: Buried's ballistic knife
                        // kills a zombie this way (BO2's `_zm_spawner` scores a
                        // knife_ballistic MOD_IMPACT kill as
                        // `ballistic_knife_death`).
                        if facts.impact_damage > 0
                            && let ColliderId::EntityDObjBone { owner, .. }
                            | ColliderId::EntityLinkedBrush { owner, .. } = collider
                            && let Some(target) = owner.script_model()
                        {
                            let bone = match collider {
                                ColliderId::EntityDObjBone { bone, .. } => Some(usize::from(bone)),
                                _ => None,
                            };
                            entity_hits.push((
                                DamageSource::Projectile(projectile.id),
                                crate::script::EntityHit {
                                    target,
                                    amount: facts.impact_damage,
                                    attacker: Some(projectile.owner),
                                    means: "MOD_IMPACT",
                                    weapon: projectile.weapon,
                                    point: end,
                                    dir: projectile.velocity,
                                    bone,
                                    flags: 0,
                                },
                            ));
                        }
                        if sticks_to_surface(facts, normal) {
                            settle_equipment(world, tick, &mut projectile, end, normal, fraction);
                        } else if !armed {
                            projectile.live = false;
                            pending_detonation = Some(PendingDetonation {
                                projectile,
                                origin: end,
                                normal,
                                surf_type: 0,
                                geometry: ProjectileHitGeometry::Dud,
                                terminal: Some(collider),
                                amount: facts.impact_damage.max(0),
                                fraction: Some(fraction),
                                surface_flags: None,
                                splash: false,
                            });
                        } else if facts.rocket_ends_on_impact() {
                            pending_detonation = Some(PendingDetonation {
                                projectile,
                                origin: end,
                                normal,
                                surf_type: 0,
                                geometry: ProjectileHitGeometry::Entity { epoch },
                                terminal: Some(collider),
                                amount: facts.impact_damage.max(0),
                                fraction: Some(fraction),
                                surface_flags: None,
                                splash: true,
                            });
                        } else {
                            bounce_missile(world, tick, &mut projectile, end, normal, fraction, 0, &step_off);
                            impacts.push(ProjectileImpact {
                                id: projectile.id,
                                owner: projectile.owner,
                                weapon: projectile.weapon,
                                origin: end,
                                geometry: ProjectileHitGeometry::Bounce,
                                terminal: Some(collider),
                                amount: 0,
                                fraction: Some(fraction),
                                surface_flags: None,
                                surf_type: None,
                                entnum: projectile.entnum,
                            });
                        }
                    }
                }
                Some(world_collider @ ColliderId::World { .. }) => {
                    let surf_type = hit_surf.unwrap_or(0);
                    if armed && facts.proj_impact_explode {
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type,
                            geometry: ProjectileHitGeometry::World,
                            terminal: Some(world_collider),
                            amount: facts.impact_damage.max(0),
                            fraction: Some(fraction),
                            surface_flags,
                            splash: true,
                        });
                    } else if sticks_to_surface(facts, normal) {
                        if let ColliderId::World { glass_encoded, .. } = world_collider
                            && glass_encoded != 0
                        {
                            let pane = u32::from(glass_encoded).saturating_sub(1);
                            if world.world_objects().glass_is_solid(pane) {
                                projectile.stuck_pane = Some(pane);
                            }
                        }
                        settle_equipment(world, tick, &mut projectile, end, normal, fraction);
                    } else if !armed {
                        projectile.live = false;
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type,
                            geometry: ProjectileHitGeometry::Dud,
                            terminal: Some(world_collider),
                            amount: facts.impact_damage.max(0),
                            fraction: Some(fraction),
                            surface_flags,
                            splash: false,
                        });
                    } else if facts.rocket_ends_on_impact() {
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type,
                            geometry: ProjectileHitGeometry::World,
                            terminal: Some(world_collider),
                            amount: 0,
                            fraction: Some(fraction),
                            surface_flags,
                            splash: true,
                        });
                    } else {
                        bounce_missile(
                            world,
                            tick,
                            &mut projectile,
                            end,
                            normal,
                            fraction,
                            surf_type,
                            &step_off,
                        );
                        impacts.push(ProjectileImpact {
                            id: projectile.id,
                            owner: projectile.owner,
                            weapon: projectile.weapon,
                            origin: end,
                            geometry: ProjectileHitGeometry::Bounce,
                            terminal: Some(world_collider),
                            amount: 0,
                            fraction: Some(fraction),
                            surface_flags,
                            surf_type: hit_surf,
                            entnum: projectile.entnum,
                        });
                    }
                }
                None => {
                    if armed && facts.proj_impact_explode {
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type: 0,
                            geometry: ProjectileHitGeometry::World,
                            terminal: None,
                            amount: facts.impact_damage.max(0),
                            fraction: Some(fraction),
                            surface_flags: None,
                            splash: true,
                        });
                    } else if !armed {
                        projectile.live = false;
                        pending_detonation = Some(PendingDetonation {
                            projectile,
                            origin: end,
                            normal,
                            surf_type: 0,
                            geometry: ProjectileHitGeometry::Dud,
                            terminal: None,
                            amount: 0,
                            fraction: Some(fraction),
                            surface_flags: None,
                            splash: false,
                        });
                    }
                }
            }
        }
    }

    // bo2zm fix list 2: a Black Ops II rolling grenade (the frag) rests and
    // rolls on its body, not on its centre.
    if pending_detonation.is_none()
        && let Some(radius) = facts.rolling_radius
        && projectile.pos.tr_type != TR_STATIONARY
    {
        // bo2mp window 5: Black Ops II rolls its bump chance each hit.
        let bump_roll = world.combat_rng_mut().next_u32() as f32 * (1.0 / 4_294_967_296.0);
        let owner = projectile.owner;
        let glass_pairs = world.world_objects().glass_damage_pairs();
        let probe = |from: [f32; 3], to: [f32; 3]| -> Option<([f32; 3], [f32; 3])> {
            match bullet_trace_with_entity_models(
                &brushes,
                &bsp,
                &cmodels,
                &mesh,
                &players,
                &script_models,
                &BulletTraceQuery {
                    start: from,
                    end: to,
                    mask: MASK_BULLET_WORLD,
                    ignore: Some(owner),
                    ignore_hit: None,
                    ignore_model: None,
                },
                &|piece| {
                    crate::world_objects::glass_piece_is_solid(
                        glass_pairs
                            .iter()
                            .find(|(id, _)| *id == u32::from(piece))
                            .map(|(_, d)| *d)
                            .unwrap_or(0),
                    )
                },
            ) {
                TraceOutcome::Hit {
                    end,
                    normal,
                    collider,
                    ..
                } if !matches!(collider, ColliderId::Player { .. }) => Some((end, normal)),
                _ => None,
            }
        };
        roll_grenade(tick, &mut projectile, radius, contact_normal, contact_vel, bump_roll, &probe);
    }

    if let Some(info) = pending_detonation {
        detonated.push(info);
    }

    if detonated.is_empty() {
        projectile.velocity = projectile.velocity_at(time);
        if let Some(row) = world.projectile_mut_by_number(entnum) {
            *row = projectile;
        }
    } else {
        let _ = world.remove_projectile_by_number(entnum);
    }
    for info in &detonated {
        impacts.push(ProjectileImpact {
            id: info.projectile.id,
            owner: info.projectile.owner,
            weapon: info.projectile.weapon,
            origin: info.origin,
            geometry: info.geometry,
            terminal: info.terminal,
            amount: info.amount,
            fraction: info.fraction,
            surface_flags: info.surface_flags,
            surf_type: info.surface_flags.map(|_| info.surf_type),
            entnum: info.projectile.entnum,
        });
    }
    world.record_projectile_impacts(tick, &impacts);
    for info in &detonated {
        if info.splash {
            world.note_dying_missile(tick, info.projectile);
        }
    }
    let resolves_damage = world.publishes_snapshot();
    if resolves_damage {
        for (projectile, target) in direct_hits {
            let facts = required_projectile_facts(world, projectile.weapon);
            let Some(target_meta) = world.client_meta(target) else {
                continue;
            };
            let intent = DamageAttempt {
                source: DamageSource::Projectile(projectile.id),
                pellet: PelletId(0),
                attacker: projectile.owner,
                attacker_life: projectile.owner_life,
                target,
                target_life: target_meta.life_sequence,
                weapon: projectile.weapon,
                amount: facts.impact_damage.max(0),
                killcam_entity_start_time: projectile_birth_ms(&projectile),
                inflictor_origin: Some(projectile.origin),
                hitloc: 0,
            };
            let _ = crate::damage::apply_damage_attempt(world, tick, &intent);
        }
        for (source, mut hit) in std::mem::take(&mut entity_hits) {
            if hit.means.is_empty() {
                hit.means = crate::script_player::means(world, source, hit.weapon, 0, false);
            }
            crate::script::damage_entity(world.ecs(), &hit);
        }
    }
    for info in detonated {
        world.weapon_notes.push(WeaponNote::Detonated {
            id: info.projectile.id,
            origin: info.origin,
        });
        // BO2's scripts get the shot's `projectile_impact` at this point.
        crate::t6::note_detonation(world.ecs(), info.projectile.id, info.origin);
        if !info.splash {
            continue;
        }
        let facts = required_projectile_facts(world, info.projectile.weapon);
        let event_kind = if facts.projectile_explosion_type == 2 {
            entity_iw4::EntityEventKind::FLASHBANG_EXPLODE
        } else {
            entity_iw4::EntityEventKind::GRENADE_EXPLODE
        };
        world.push_entity_event(
            tick,
            EventAudience::All,
            event_kind,
            crate::EntityEventPayload {
                number: info.projectile.entnum,
                attacker_entity_num: info.projectile.owner.0 as i32,
                weapon: info.projectile.weapon,
                correlation: info.projectile.id.0,
                origin: info.origin,
                direction: info.normal,
                surf_type: info.surf_type,
                ..Default::default()
            },
        );
        if !resolves_damage {
            continue;
        }
        if facts.projectile_explosion_type == 2 {
            crate::damage::apply_flashbang_blast(
                world,
                info.origin,
                facts.explosion_radius.max(0) as f32,
                facts.explosion_radius_min.max(0) as f32,
                info.projectile.owner,
            );
        }
        let radius = facts
            .explosion_radius
            .max(facts.explosion_radius_min)
            .max(0) as f32;
        if radius <= 0.0 || (facts.explosion_inner_damage <= 0 && facts.explosion_outer_damage <= 0)
        {
            continue;
        }
        let blast = crate::damage::ExplosionBlast {
            cone: (facts.damage_cone_angle > 0.0 && facts.damage_cone_angle < 180.0).then(|| {
                (
                    forward(info.projectile.apos.tr_base),
                    facts.damage_cone_angle.to_radians().cos(),
                )
            }),
            origin: info.origin,
            radius,
            inner_damage: facts.explosion_inner_damage as f32,
            outer_damage: facts.explosion_outer_damage.max(0) as f32,
            weapon: info.projectile.weapon,
            source: DamageSource::Projectile(info.projectile.id),
            attacker: info.projectile.owner,
            attacker_life: info.projectile.owner_life,
            killcam_entity_start_time: projectile_birth_ms(&info.projectile),
        };
        crate::damage::apply_explosion_blast(world, tick, &blast);
        crate::damage::apply_shared_glass_blast(
            world,
            tick,
            info.origin,
            facts.explosion_inner_damage,
            facts.explosion_outer_damage,
            radius,
        );
    }
}

fn push_grenade_bounce(
    world: &mut FrameWorld,
    tick: Tick,
    projectile: &ProjectileState,
    surf_type: u8,
) {
    world.push_entity_event(
        tick,
        EventAudience::All,
        entity_iw4::EntityEventKind::GRENADE_BOUNCE,
        crate::EntityEventPayload {
            number: projectile.entnum,
            weapon: projectile.weapon,
            correlation: projectile.id.0,
            origin: projectile.origin,
            event_parm: i32::from(surf_type),
            surf_type,
            ..Default::default()
        },
    );
}

fn push_grenade_stick(
    world: &mut FrameWorld,
    tick: Tick,
    projectile: &ProjectileState,
    player: Option<ClientId>,
    shield: bool,
) {
    world.weapon_notes.push(WeaponNote::Stuck {
        id: projectile.id,
        player,
        shield,
    });
    world.push_entity_event(
        tick,
        EventAudience::All,
        entity_iw4::EntityEventKind::GRENADE_STICK,
        crate::EntityEventPayload {
            number: projectile.entnum,
            weapon: projectile.weapon,
            correlation: projectile.id.0,
            origin: projectile.origin,
            ..Default::default()
        },
    );
}

fn missile_glass_punch_eligible(speed: f32, surface_flags: u32, glass_encoded: u16) -> bool {
    speed >= MISSILE_GLASS_SHATTER_VEL
        && surface_type_from_flags(surface_flags) == SURF_TYPE_GLASS
        && glass_encoded != 0
}

fn park_missile_at(
    tick: Tick,
    projectile: &mut ProjectileState,
    origin: [f32; 3],
    velocity: [f32; 3],
) {
    let time = level_time_ms(tick);
    projectile.origin = origin;
    projectile.velocity = velocity;
    projectile.pos.tr_base = origin;
    projectile.pos.tr_time = time;
    projectile.pos.tr_delta = velocity;
}

fn sticks_to_surface(facts: EquipmentRuntimeFacts, normal: [f32; 3]) -> bool {
    matches!(facts.stickiness, 1 | 2) || (matches!(facts.stickiness, 3 | 4) && normal[2] > 0.7)
}

fn settle_equipment(
    world: &mut FrameWorld,
    tick: Tick,
    projectile: &mut ProjectileState,
    origin: [f32; 3],
    normal: [f32; 3],
    fraction: f32,
) {
    let facts = required_projectile_facts(world, projectile.weapon);
    let yaw = evaluate_trajectory(&projectile.apos, level_time_ms(tick))[1];
    if facts.stickiness == 3 {
        apply_missile_land_angles(world, tick, projectile, normal, fraction);
    }
    let origin = core::array::from_fn(|i| origin[i] + normal[i] * 0.25);
    stick_missile(tick, projectile, origin);
    if matches!(facts.stickiness, 2 | 4) {
        let mut angles = projectile.apos.tr_base;
        if facts.stickiness == 4 {
            angles = [math_iw4::pitch_for_yaw_on_normal(yaw, normal), yaw, 0.0];
        } else {
            let facing = forward(angles);
            let along: f32 = (0..3).map(|i| facing[i] * normal[i]).sum();
            angles =
                math_iw4::vect_to_angles(core::array::from_fn(|i| facing[i] - along * normal[i]));
        }
        let (_, right, up) = math_iw4::angle_vectors(angles);
        let side: f32 = (0..3).map(|i| normal[i] * right[i]).sum();
        let vertical: f32 = (0..3).map(|i| normal[i] * up[i]).sum();
        angles[2] = side.atan2(vertical).to_degrees();
        projectile.apos.tr_base = angles;
    }
    if !facts.timed_detonation {
        projectile.detonate_at_ms = None;
        projectile.cleanup_at_ms = i32::MAX;
    }
    push_grenade_stick(world, tick, projectile, None, false);
}

fn stick_missile(tick: Tick, projectile: &mut ProjectileState, origin: [f32; 3]) {
    let time = level_time_ms(tick);
    projectile.origin = origin;
    projectile.velocity = [0.0, 0.0, 0.0];
    projectile.pos.tr_type = TR_STATIONARY;
    projectile.pos.tr_time = time;
    projectile.pos.tr_duration = 0;
    projectile.pos.tr_base = origin;
    projectile.pos.tr_delta = [0.0, 0.0, 0.0];
    projectile.apos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: time,
        tr_duration: 0,
        tr_base: evaluate_trajectory(&projectile.apos, time),
        tr_delta: [0.0; 3],
    };
}

fn knife_impact(
    world: &mut FrameWorld,
    tick: Tick,
    projectile: &mut ProjectileState,
    normal: [f32; 3],
    fraction: f32,
    surf_type: u8,
    player_hit: bool,
) {
    let time = level_time_ms(tick);
    let hit_time =
        time - crate::MATCH_TICK_MS as i32 + (crate::MATCH_TICK_MS as f32 * fraction) as i32;
    let facts = required_projectile_facts(world, projectile.weapon);
    if facts.ballistic_blade {
        let origin = core::array::from_fn(|i| projectile.origin[i] + normal[i] * 0.25);
        stick_missile(tick, projectile, origin);
        push_grenade_stick(world, tick, projectile, None, false);
        return;
    }
    projectile.velocity = projectile.velocity_at(hit_time);
    bounce_velocity(projectile, normal, &facts, surf_type);
    let speed = vec3_length(projectile.velocity);
    let direction = vec3_normalize(projectile.velocity).unwrap_or([0.0; 3]);
    let incidence: f32 = (0..3).map(|i| direction[i] * normal[i]).sum();
    let floor = normal[2] > 0.7;
    let stop = (player_hit && facts.stick_to_players)
        || (!player_hit && ((floor && speed < 20.0) || incidence > 0.7));
    if !stop {
        projectile.origin = core::array::from_fn(|i| {
            projectile.origin[i]
                + if i == 2 {
                    (normal[i] * 0.1).min(0.0)
                } else {
                    normal[i] * 0.1
                }
        });
        projectile.pos.tr_base = projectile.origin;
        projectile.pos.tr_time = time;
        projectile.pos.tr_delta = projectile.velocity;
        apply_missile_land_angles(world, tick, projectile, normal, fraction);
        push_grenade_bounce(world, tick, projectile, surf_type);
        return;
    }
    let mut angles = evaluate_trajectory(&projectile.apos, hit_time);
    let mut origin = projectile.origin;
    if !player_hit && floor && (speed < 20.0 || incidence < 0.7) {
        let forward = forward(angles);
        let dot: f32 = (0..3).map(|i| forward[i] * normal[i]).sum();
        angles = math_iw4::vect_to_angles(core::array::from_fn(|i| forward[i] - dot * normal[i]));
        let (_, right, up) = math_iw4::angle_vectors(angles);
        let side: f32 = (0..3).map(|i| normal[i] * right[i]).sum();
        let vertical: f32 = (0..3).map(|i| normal[i] * up[i]).sum();
        angles[2] = side.atan2(vertical).to_degrees() + 90.0;
        origin[2] -= 1.0;
    } else {
        angles[0] = normal[2].atan2(normal[0].hypot(normal[1])).to_degrees()
            + host_flrand(world.combat_rng_mut(), -15.0, 15.0);
        let forward = forward(angles);
        origin = core::array::from_fn(|i| origin[i] - normal[i] * 1.5 - forward[i] * 4.5);
    }
    stick_missile(tick, projectile, origin);
    projectile.apos.tr_base = angles;
    push_grenade_stick(world, tick, projectile, None, false);
}

fn unstick_missile(tick: Tick, projectile: &mut ProjectileState) {
    let time = level_time_ms(tick);
    projectile.velocity = [0.0, 0.0, 0.0];
    projectile.pos.tr_type = TR_GRAVITY;
    projectile.pos.tr_time = time;
    projectile.pos.tr_duration = 0;
    projectile.pos.tr_base = projectile.origin;
    projectile.pos.tr_delta = [0.0, 0.0, 0.0];
}

fn apply_missile_land_angles(
    world: &mut FrameWorld,
    tick: Tick,
    projectile: &mut ProjectileState,
    normal: [f32; 3],
    fraction: f32,
) {
    let time = level_time_ms(tick);
    let prev = time.saturating_sub(crate::MATCH_TICK_MS as i32);
    let hit_time = prev.saturating_add(((time - prev) as f32 * fraction) as i32);
    let (spin_random, wall_spin_addend) = {
        let rng = world.combat_rng_mut();
        let spin_random = rng.next_u32() as f32 * (1.0 / 4_294_967_296.0);
        let wall_spin_addend = ((rng.next_u32() & 0x7f) as i32 - 63) as f32;
        (spin_random, wall_spin_addend)
    };
    projectile.apos = missile_land_angles(MissileLandAnglesIn {
        apos: projectile.apos,
        normal,
        hit_time_ms: hit_time,
        force_align: false,
        spin_random,
        wall_spin_addend,
    })
    .apos;
}

fn bounce_missile(
    world: &mut FrameWorld,
    tick: Tick,
    projectile: &mut ProjectileState,
    origin: [f32; 3],
    normal: [f32; 3],
    fraction: f32,
    surf_type: u8,
    step_off: &dyn Fn([f32; 3], [f32; 3]) -> f32,
) {
    let time = level_time_ms(tick);
    let prev = time.saturating_sub(crate::MATCH_TICK_MS as i32);
    let hit_time = prev.saturating_add(((time - prev) as f32 * fraction) as i32);
    projectile.grounded |= normal[2] > 0.7;
    projectile.velocity = projectile.velocity_at(hit_time);
    let incoming = projectile.velocity;
    let facts = required_projectile_facts(world, projectile.weapon);
    bounce_velocity(projectile, normal, &facts, surf_type);
    let outgoing = projectile.velocity;
    // bo2mp window 5: Black Ops II then moves a bounced grenade a tenth of a unit along the
    // surface's normal, never up, as far as nothing blocks it, so its next move starts clear
    // of what it hit.
    let target = [
        origin[0] + normal[0] * 0.1,
        origin[1] + normal[1] * 0.1,
        origin[2] + (normal[2] * 0.1).min(0.0),
    ];
    let clear = step_off(origin, target).clamp(0.0, 1.0);
    let origin: [f32; 3] = core::array::from_fn(|i| origin[i] + (target[i] - origin[i]) * clear);
    projectile.origin = origin;
    projectile.pos.tr_base = origin;
    projectile.pos.tr_time = time;
    projectile.pos.tr_delta = projectile.velocity;
    apply_missile_land_angles(world, tick, projectile, normal, fraction);
    let delta = vec3_length([
        outgoing[0] - incoming[0],
        outgoing[1] - incoming[1],
        outgoing[2] - incoming[2],
    ]);
    if delta > BOUNCE_EVENT_SPEED_DELTA {
        push_grenade_bounce(world, tick, projectile, surf_type);
    }
    // BO2's BounceMissile: a non-sticky grenade off a floor slower than
    // grenadeRestThreshold, or all but stopped anywhere, comes to rest there
    // (`stationary`); one without a timed fuse (the Care Package and AGR
    // markers) then never goes off by itself (nextthink = 0).
    let speed = vec3_length(outgoing);
    if facts.stickiness == 0
        && facts.rolling_radius.is_none()
        && (speed < 1.0 || (normal[2] > 0.7 && speed < GRENADE_REST_THRESHOLD))
    {
        stick_missile(tick, projectile, origin);
        if !facts.timed_detonation {
            projectile.detonate_at_ms = None;
            projectile.cleanup_at_ms = i32::MAX;
        }
    }
}

/// BO2's grenadeRestThreshold dvar default (g.c G_RegisterDvars).
const GRENADE_REST_THRESHOLD: f32 = 20.0;

/// bo2zm fix list 2: under its body: how far below its resting height a
/// rolling grenade still finds the ground it rolls on.
const ROLL_SNAP: f32 = 2.0;
/// bo2mp window 5: Black Ops II's rolling grenade rules (its engine defaults):
/// a hit slower than this along the surface normal rolls instead of bouncing.
const ROLL_HIT_SPEED: f32 = 25.0;
/// Speed along the ground below which each hit takes the low friction.
const ROLL_FRICTION_THRESH: f32 = 100.0;
/// Fraction of its speed a hit takes away: slow (rolling) and fast.
const ROLL_FRICTION_LOW: f32 = 0.01;
const ROLL_FRICTION_HIGH: f32 = 0.4;
/// The most of a hard hit's normal speed it bounces back with on a wall.
const ROLL_RESTITUTION_MAX: f32 = 0.3;
/// Slower than this on a floor, it comes to rest.
const ROLL_REST_SPEED: f32 = 20.0;
/// Bumps: chance per hit, size (fraction of its speed) and the cap.
const ROLL_BUMP_CHANCE: f32 = 0.3;
const ROLL_BUMP_MAG: f32 = 0.4;
const ROLL_BUMP_MAX: f32 = 100.0;
/// Wobble: cycles per unit rolled, forward speed swing, side swing, and how
/// fast the side swing fades with speed.
const ROLL_WOBBLE_FREQ: f32 = 0.08;
const ROLL_WOBBLE_FWD: f32 = 10.0;
const ROLL_WOBBLE_SIDE: f32 = 1.0;
const ROLL_WOBBLE_SIDE_DAMP: f32 = 0.01;
/// The most sideways speed its curve adds per hit (each grenade curves its
/// own random way between minus and plus this).
const ROLL_CURVE_MAX: f32 = 4.0;
/// The radius its spin is drawn for.
const ROLL_SPIN_RADIUS: f32 = 5.0;
const ROLL_GRAVITY: f32 = 800.0;
const ROLL_TICK_S: f32 = crate::MATCH_TICK_MS as f32 / 1000.0;

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// bo2mp window 5: a Black Ops II rolling grenade's hit (the frag, flash,
/// concussion, EMP): its speed into the surface is gone, each hit takes a
/// share of its speed (a little when slow, a lot when fast), and only a hard
/// hit (25 units/s or more into it) bounces back, by its perpendicular
/// bounce, capped by how upright the surface is.
fn bo2_rolling_bounce(incoming: [f32; 3], normal: [f32; 3], perpendicular: f32) -> [f32; 3] {
    let Some(n) = vec3_normalize(normal) else {
        return [0.0, 0.0, 0.0];
    };
    let into = dot3(incoming, n);
    let mut v: [f32; 3] = core::array::from_fn(|i| incoming[i] - into * n[i]);
    if vec3_length(incoming) <= 0.0 || into > 0.0 {
        return v;
    }
    v = roll_friction(v);
    if into.abs() >= ROLL_HIT_SPEED {
        let back = (1.0 - (1.0 - perpendicular) * n[2]).min(ROLL_RESTITUTION_MAX.max(perpendicular));
        v = core::array::from_fn(|i| v[i] - back * into * n[i]);
    }
    v
}

fn roll_friction(v: [f32; 3]) -> [f32; 3] {
    let friction = if vec3_length(v) > ROLL_FRICTION_THRESH {
        ROLL_FRICTION_HIGH
    } else {
        ROLL_FRICTION_LOW
    };
    v.map(|c| c * (1.0 - friction))
}

/// Each grenade's own curve, between -1 and 1, fixed for its life.
fn roll_curve(projectile: &ProjectileState) -> f32 {
    let h = (projectile.spawn_time_ms as u32).wrapping_mul(2_654_435_761)
        ^ (projectile.entnum as u32).wrapping_mul(40_503);
    let h = h ^ (h >> 15);
    (h >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
}

/// bo2zm fix list 2 + bo2mp window 5: one tick of a Black Ops II rolling
/// grenade (`isRollingGrenade`) after the shared bounce ran. Whatever it
/// touched it touched with its body (`radius` off the surface, not its
/// centre on it). On a floor it hits the ground every tick, as Black Ops II's
/// does under gravity: its speed into the ground is gone, gravity pulls it
/// down a slope, each hit takes its friction, now and then it bumps up, it
/// wobbles and curves, and it spins as it rolls, until it is slower than its
/// rest speed or rolls off an edge and falls. `pre_vel` is its velocity
/// when it touched, before this tick's bounce; `bump_roll` is a random 0..1 for this tick.
/// True when it came to rest.
fn roll_grenade(
    tick: Tick,
    projectile: &mut ProjectileState,
    radius: f32,
    contact_normal: Option<[f32; 3]>,
    pre_vel: [f32; 3],
    bump_roll: f32,
    probe: &dyn Fn([f32; 3], [f32; 3]) -> Option<([f32; 3], [f32; 3])>,
) -> bool {
    let time = level_time_ms(tick);
    let rolling = projectile.pos.tr_type == entity_iw4::TR_LINEAR;
    let mut origin = projectile.origin;
    let mut v = projectile.velocity_at(time);
    if let Some(n) = contact_normal {
        origin = core::array::from_fn(|i| origin[i] + n[i] * radius);
        // Rolling onto a rise is a hit like any other tick's.
        if rolling && n[2] > 0.7 {
            v = pre_vel;
        }
    }
    let angles = evaluate_trajectory(&projectile.apos, time);
    let mut hit_this_tick = rolling;
    if !rolling {
        let floor = contact_normal.filter(|n| n[2] > 0.7);
        let Some(n) = floor else {
            // Still in the air, or off a wall: just off what it hit.
            if contact_normal.is_some() {
                projectile.origin = origin;
                projectile.pos.tr_base = origin;
            }
            return false;
        };
        if vec3_length(v) < ROLL_REST_SPEED {
            stick_missile(tick, projectile, origin);
            return true;
        }
        if dot3(pre_vel, n).abs() >= ROLL_HIT_SPEED {
            // A hard landing: it bounced. It hops from where it touched, so
            // it comes down as fast as it went up, as Black Ops II's does;
            // lifting it by its body here would add a fall to every hop and
            // keep it hopping (never rolling) on flat ground.
            return false;
        }
        // It landed softly: the bounce already took this tick's hit.
        hit_this_tick = false;
    }
    let below = [origin[0], origin[1], origin[2] - (radius + ROLL_SNAP)];
    let Some((ground, n)) = probe(origin, below).filter(|(_, n)| n[2] > 0.7) else {
        // Rolled off an edge: it falls.
        projectile.origin = origin;
        projectile.pos = Trajectory {
            tr_type: TR_GRAVITY,
            tr_time: time,
            tr_duration: 0,
            tr_base: origin,
            tr_delta: v,
        };
        return false;
    };
    origin = core::array::from_fn(|i| ground[i] + n[i] * radius);
    // bo2mp window 5: rolling on a floor is being on the ground, so a
    // ground-only fuse (EMP, concussion, smoke) goes off as it rolls or rests;
    // the flight step clears it every tick a rolling grenade moves.
    projectile.grounded = true;
    // Its speed into the ground is gone; gravity pulls it down the slope.
    let g = [0.0, 0.0, -ROLL_GRAVITY];
    let gn = dot3(g, n);
    let vn = dot3(v, n);
    v = core::array::from_fn(|i| v[i] - n[i] * vn + (g[i] - n[i] * gn) * ROLL_TICK_S);
    let speed = vec3_length(v);
    let mut airborne = false;
    if hit_this_tick && speed > 0.0 {
        let dir = v.map(|c| c / speed);
        v = roll_friction(v);
        if bump_roll < ROLL_BUMP_CHANCE {
            let bump = (ROLL_BUMP_MAG * speed).min(ROLL_BUMP_MAX);
            if bump > ROLL_GRAVITY * 0.05 {
                v = core::array::from_fn(|i| v[i] + n[i] * bump);
                airborne = true;
            }
        }
        // The wobble's phase is the distance rolled (the tick's move already
        // added it), so it turns at its cycles per unit as Black Ops II's does.
        let tau = core::f32::consts::TAU;
        let phase = (projectile.travel_distance * ROLL_WOBBLE_FREQ * tau) % tau;
        let next = phase + ROLL_WOBBLE_FREQ * speed * ROLL_TICK_S * tau;
        let swing = next.sin() - phase.sin();
        if speed > 0.01 {
            let scale = (ROLL_WOBBLE_FWD * swing + speed) / speed;
            v = v.map(|c| c * scale);
        }
        let side = cross3(dir, n);
        let damp = 1.0 - ROLL_WOBBLE_SIDE_DAMP * speed;
        if damp > 0.0 {
            let off = ROLL_WOBBLE_SIDE * swing * damp;
            let to: [f32; 3] = core::array::from_fn(|i| origin[i] + side[i] * off);
            origin = probe(origin, to).map_or(to, |(end, _)| end);
        }
        let curve = ROLL_CURVE_MAX * roll_curve(projectile);
        v = core::array::from_fn(|i| v[i] + side[i] * curve);
    }
    let speed = vec3_length(v);
    if !airborne && speed < ROLL_REST_SPEED {
        stick_missile(tick, projectile, origin);
        projectile.apos.tr_base = angles;
        return true;
    }
    projectile.origin = origin;
    projectile.velocity = v;
    projectile.pos = Trajectory {
        tr_type: if airborne {
            TR_GRAVITY
        } else {
            entity_iw4::TR_LINEAR
        },
        tr_time: time,
        tr_duration: 0,
        tr_base: origin,
        tr_delta: v,
    };
    // It spins about the axis across its path, once per turn of its spin radius.
    let yaw = v[1].atan2(v[0]).to_degrees();
    projectile.apos = Trajectory {
        tr_type: entity_iw4::TR_LINEAR,
        tr_time: time,
        tr_duration: 0,
        tr_base: [angles[0], yaw, 90.0],
        tr_delta: [speed / (ROLL_SPIN_RADIUS * core::f32::consts::TAU) * 360.0, 0.0, 0.0],
    };
    false
}

fn required_projectile_facts(world: &FrameWorld, weapon: u32) -> EquipmentRuntimeFacts {
    world
        .equipment_facts_for(weapon)
        .or_else(|| world.missile_launch_facts(weapon))
        .unwrap_or_else(|| {
            panic!(
                "a missile needs a validated equipment row; bounce/damage must not default to zero"
            );
        })
}

/// A fired projectile's (impact damage, blast radius) for the BO2 scripts'
/// `projectile_impact` and the models its path goes through.
pub(crate) fn projectile_impact_facts(world: &FrameWorld, weapon: u32) -> Option<(i32, f32)> {
    let f = world
        .equipment_facts_for(weapon)
        .or_else(|| world.missile_launch_facts(weapon))?;
    Some((
        f.impact_damage.max(0),
        f.explosion_radius.max(f.explosion_radius_min).max(0) as f32,
    ))
}

fn bounce_velocity(
    projectile: &mut ProjectileState,
    normal: [f32; 3],
    facts: &EquipmentRuntimeFacts,
    surf_type: u8,
) {
    let coefficients = facts
        .parallel_bounce
        .as_ref()
        .zip(facts.perpendicular_bounce.as_ref())
        .and_then(|(p, n)| p.get(surf_type as usize).zip(n.get(surf_type as usize)))
        .unwrap_or_else(|| panic!("missing projectile surface bounce coefficients"));
    let incoming = projectile.velocity;
    projectile.velocity = if facts.projectile_parent_velocity.is_some()
        && facts.rolling_radius.is_some()
    {
        bo2_rolling_bounce(incoming, normal, *coefficients.1)
    } else if facts.projectile_parent_velocity.is_some() {
        bo2_bounce(incoming, normal, *coefficients.0, *coefficients.1)
    } else {
        bounce_reflected_scale(incoming, normal, *coefficients.0, *coefficients.1)
    };
}

/// bo2mp window 5: a Black Ops II bounce (its rows are the ones with a parent
/// velocity field): mirror off the surface, then scale the part along the
/// surface by `parallel` and the part off it by `perpendicular`, each on its
/// own. One blended factor on the whole mirror kept too much of a head-on hit
/// and too little of a glancing one.
fn bo2_bounce(incoming: [f32; 3], normal: [f32; 3], parallel: f32, perpendicular: f32) -> [f32; 3] {
    let Some(n) = vec3_normalize(normal) else {
        return [0.0, 0.0, 0.0];
    };
    let d = dot3(incoming, n);
    let mirrored: [f32; 3] = core::array::from_fn(|i| incoming[i] - 2.0 * d * n[i]);
    let off = dot3(mirrored, n);
    core::array::from_fn(|i| {
        let off_part = n[i] * off;
        (mirrored[i] - off_part) * parallel + off_part * perpendicular
    })
}

fn bounce_reflected_scale(
    incoming: [f32; 3],
    normal: [f32; 3],
    parallel: f32,
    perpendicular: f32,
) -> [f32; 3] {
    let speed = vec3_length(incoming);
    let nlen = vec3_length(normal);
    if speed < 1e-6 || nlen < 1e-6 {
        return [0.0, 0.0, 0.0];
    }
    let n = [normal[0] / nlen, normal[1] / nlen, normal[2] / nlen];
    let d = incoming[0] * n[0] + incoming[1] * n[1] + incoming[2] * n[2];
    let reflected = [
        incoming[0] - 2.0 * d * n[0],
        incoming[1] - 2.0 * d * n[1],
        incoming[2] - 2.0 * d * n[2],
    ];
    let incidence = (-d / speed).clamp(0.0, 1.0);
    let factor = parallel + (perpendicular - parallel) * incidence;
    [
        reflected[0] * factor,
        reflected[1] * factor,
        reflected[2] * factor,
    ]
}

/// bo2mp window 5: Black Ops II's missile trace against a player's hit boxes. A grenade that
/// starts inside a hit box and leaves it passes out freely; one that stays inside it while it
/// moves in toward the player's middle stops where it is, facing level and straight out from
/// his middle. (Our shared trace calls any start inside a hit box a start inside something.)
fn missile_trace(
    players: &[crate::PlayerCollisionPose],
    query: &BulletTraceQuery,
    trace: &dyn Fn(&[crate::PlayerCollisionPose], &BulletTraceQuery) -> TraceOutcome,
) -> TraceOutcome {
    let outcome = trace(players, query);
    let TraceOutcome::StartSolid {
        collider: Some(ColliderId::Player { .. }),
        ..
    } = outcome
    else {
        return outcome;
    };
    let inside = |bone: &crate::AuthorityDObjCollisionBone, point: [f32; 3]| {
        let delta: [f32; 3] = core::array::from_fn(|i| point[i] - bone.center[i]);
        (0..3).all(|axis| dot3(delta, bone.axes[axis]).abs() <= bone.half_size[axis])
    };
    let along = [query.end[0] - query.start[0], query.end[1] - query.start[1]];
    let mut kept = players.to_vec();
    for pose in &mut kept {
        if query.ignore == Some(pose.client) || query.ignore_hit == Some(pose.client) {
            continue;
        }
        let out = [query.start[0] - pose.origin[0], query.start[1] - pose.origin[1]];
        if along[0] * out[0] + along[1] * out[1] < 0.0
            && let Some(bone) = pose
                .bones
                .iter()
                .find(|bone| inside(bone, query.start) && inside(bone, query.end))
        {
            let len = out[0].hypot(out[1]);
            let normal = if len > 0.0 {
                [out[0] / len, out[1] / len, 0.0]
            } else {
                [0.0; 3]
            };
            return TraceOutcome::Hit {
                fraction: 0.0,
                end: query.start,
                normal,
                collider: ColliderId::Player {
                    client: pose.client,
                    life: pose.life_sequence,
                    hitloc: bone.part_classification,
                },
            };
        }
        pose.bones.retain(|bone| !inside(bone, query.start));
    }
    trace(&kept, query)
}

fn startsolid_normal(start: [f32; 3], end: [f32; 3]) -> [f32; 3] {
    let d = [start[0] - end[0], start[1] - end[1], start[2] - end[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    // bo2mp window 5: no move means no facing (Black Ops II leaves it zero).
    if len < 1e-6 {
        [0.0, 0.0, 0.0]
    } else {
        [d[0] / len, d[1] / len, d[2] / len]
    }
}

fn entity_collision_epoch(
    terminal: ColliderId,
    rows: &[EntityCollisionTraceGeom],
) -> Option<EntityCollisionEpoch> {
    let owner = match terminal {
        ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. } => {
            owner
        }
        ColliderId::World { .. } | ColliderId::Player { .. } => return None,
    };
    rows.iter()
        .find(|row| row.owner == owner)
        .map(|row| row.epoch)
}

#[cfg(test)]
mod roll_tests {
    use super::*;

    /// A flat floor at z = 0.
    fn floor(from: [f32; 3], to: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
        if to[2] > 0.0 {
            return None;
        }
        let t = from[2] / (from[2] - to[2]);
        Some((
            core::array::from_fn(|i| from[i] + (to[i] - from[i]) * t),
            [0.0, 0.0, 1.0],
        ))
    }

    /// A frag (body 1.37) rolling along +x at `speed`; each tick moves it along
    /// its line and gives it one hit. Returns (tick, distance rolled, speed)
    /// until it rests. It curves its own way, so the distance is along its path.
    fn roll(speed: f32) -> Vec<(u32, f32, f32)> {
        let origin = [0.0, 0.0, 1.37];
        let pos = Trajectory {
            tr_type: entity_iw4::TR_LINEAR,
            tr_time: 0,
            tr_duration: 0,
            tr_base: origin,
            tr_delta: [speed, 0.0, 0.0],
        };
        let mut p = ProjectileState {
            id: ProjectileId(1),
            owner: ClientId(0),
            owner_life: LifeSequence(0),
            weapon: 1,
            origin,
            velocity: pos.tr_delta,
            pos,
            apos: Trajectory::default(),
            entnum: 70,
            launch_time: 0,
            spawn_time_ms: 0,
            detonate_at_ms: None,
            cleanup_at_ms: i32::MAX,
            travel_distance: 0.0,
            live: true,
            stuck_pane: None,
            grounded: false,
            guide: crate::MissileGuide::default(),
            impact_target: None,
        };
        let mut out = vec![(0, 0.0, speed)];
        for k in 1..400 {
            let tick = Tick(k);
            let time = level_time_ms(tick);
            let from = p.origin;
            p.origin = p.origin_at(time);
            // The tick's move counts as distance rolled, as in the game.
            p.travel_distance += vec3_length(core::array::from_fn(|i| p.origin[i] - from[i]));
            let vel = p.velocity_at(time);
            // The flight step clears the ground every tick it moves; rolling
            // on the floor must set it again, or a ground-only fuse (EMP)
            // never goes off.
            p.grounded = false;
            // No bumps, so the decay reads cleanly.
            let rested = roll_grenade(tick, &mut p, 1.37, None, vel, 1.0, &floor);
            assert!(p.grounded, "rolling on the floor is not grounded at tick {k}");
            if rested {
                out.push((k, p.travel_distance, 0.0));
                return out;
            }
            out.push((k, p.travel_distance, vec3_length(p.velocity)));
        }
        out
    }

    #[test]
    fn bo2_roll_friction_rest_and_distance() {
        let t = roll(300.0);
        if std::env::var_os("ROLL_TRACE").is_some() {
            for (k, x, s) in &t {
                println!("tick {k:3} t {:5.2}s rolled {x:7.1} speed {s:6.1}", *k as f32 * 0.05);
            }
        }
        // Above 100 units/s a hit keeps 0.6 of it (wobble adds up to +-10).
        assert!((t[1].2 - 180.0).abs() < 12.0, "{:?}", &t[..3]);
        assert!((t[2].2 - 108.0).abs() < 12.0, "{:?}", &t[..3]);
        // At or below 100 a hit keeps 0.99, and the wobble swings it by up to
        // about +-20 around that, so it never speeds back up past where it was.
        let rest = t.iter().position(|e| e.2 == 0.0).expect("never rested");
        assert!(t[3..rest].iter().all(|e| e.2 < 75.0), "{:?}", &t[3..rest.min(12)]);
        // A wobble's low takes it under the rest speed (20) once its mean is in
        // the 20s: about 1.5 s and 80 units along its path for this one.
        let (k, x, _) = t[rest];
        assert!((20..=80).contains(&k), "rested at tick {k}");
        assert!((50.0..=200.0).contains(&x), "rolled {x}");
    }
}
