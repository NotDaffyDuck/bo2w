use crate::frame::FrameWorld;
use crate::world::ClientId;
use crate::{AuthorityDObjState, AuthorityModelOwner, EventAudience, Tick};
use crate::world_objects::{
    T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH, T6_MOD_IMPACT, T6_MOD_MELEE, T6_MOD_PROJECTILE_SPLASH,
    T6_MOD_UNKNOWN,
};
use std::sync::Arc;
use xmodel_runtime::{RetainedModelCapability, T5DestructibleDef, T5DestructiblePiece};

/// bo2mp: BO2's `meansOfDeath_t` numbers a prop reads that `world_objects`
/// has no name for.
const MOD_GRENADE: u8 = 3;
const MOD_PROJECTILE: u8 = 5;
const MOD_BURNED: u8 = 17;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct State {
    definition: Arc<T5DestructibleDef>,
    health: Vec<i16>,
    /// bo2mp: a Black Ops II map's scripts hear its breaks (`notices`,
    /// t6/destructible.rs).
    hosted: bool,
    notices: Vec<Notice>,
}

/// bo2mp: what a break tells a Black Ops II map's scripts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    /// `codecallback_destructibleevent("broken", notify, attacker, weapon)`:
    /// a stage with a break notify broke (a mannequin's "headless").
    Broken {
        notify: String,
        attacker: Option<ClientId>,
        weapon: u32,
    },
    /// `codecallback_destructibleevent("breakafter", piece, time, damage)`:
    /// a stage that breaks on its own after `time` seconds (a burning car).
    BreakAfter { piece: usize, time: f32, damage: i32 },
    /// The base piece is destroyed (by `attacker`): a loose prop's (ours)
    /// end. BO2's own props die by the entity's health (`Damage`).
    BaseDestroyed { attacker: Option<ClientId> },
    /// BO2's G_Damage on the entity: its health loses `amount`, then it
    /// hears "damage" (and "death" once the health is gone).
    Damage {
        amount: i32,
        attacker: Option<ClientId>,
        means: u8,
        weapon: u32,
        point: [f32; 3],
        dir: Option<[f32; 3]>,
        flags: i32,
    },
    /// The base piece's new stage: the entity's loop (a burning car's fire),
    /// none to stop it.
    Loop(Option<String>),
}

/// bo2mp: which of a piece's damage scales a hit counts by, and how far a
/// piece it breaks off flies (a Black Ops II map's prop takes its rules from
/// the means of death, BO2's `DestructibleDamage`, `DamagePiece`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DamageKind {
    Bullet,
    /// A grenade's or a projectile's own hit.
    Explosive,
    /// `MOD_EXPLOSIVE`: it also breaks past a stage that stops everything else.
    Blast,
    /// A grenade's or a projectile's splash: it breaks past as a blast does,
    /// but struck straight (not in a blast's radius) counts as a bullet does.
    Splash,
    /// Only breaks a stage that allows it.
    Melee,
    /// A thrown or fired object's impact (`MOD_IMPACT`).
    Impact,
    /// Fire: every piece takes a random part of it and passes none to the
    /// pieces under it.
    Burned,
    /// Any other means (a script's `dodamage`, a crush): it sends no piece
    /// flying.
    Script,
}

pub fn install(dobj: &mut AuthorityDObjState, definition: Arc<T5DestructibleDef>) {
    dobj.t5_destructible = Some(State {
        health: definition.pieces.iter().map(|p| p.health as i16).collect(),
        definition,
        hosted: false,
        notices: Vec::new(),
    });
}

/// bo2mp: a Black Ops II map's destructible: every piece at full health,
/// the bones the later stages show hidden (a mannequin's broken neck), its
/// breaks kept for the scripts.
pub(crate) fn install_hosted(dobj: &mut AuthorityDObjState, definition: Arc<T5DestructibleDef>) {
    install(dobj, definition);
    if let Some(state) = &mut dobj.t5_destructible {
        state.hosted = true;
    }
    update_hide_parts(dobj);
}

/// bo2mp: whether `dobj` breaks by the definition `name`.
pub(crate) fn installed(dobj: &AuthorityDObjState, name: &str) -> bool {
    dobj.t5_destructible
        .as_ref()
        .is_some_and(|s| s.definition.name.eq_ignore_ascii_case(name))
}

/// bo2mp: whether `owner` is a Black Ops II map's loose prop (a clipmap
/// dynent, `dyn_<i>`, t6_m2.rs). BO2's client hurts one by the gun's full
/// damage (`damage[0]`, DynEntCl_DynEntBulletImpactEvent), never less for
/// range or for what the bullet went through.
pub(crate) fn is_loose_prop(world: &FrameWorld, owner: AuthorityModelOwner) -> bool {
    world
        .entity_collision_capabilities()
        .iter()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_ref())
        .and_then(|d| d.t5_destructible.as_ref())
        .is_some_and(|s| s.definition.name.starts_with("dyn_"))
}

/// bo2mp: the breaks the scripts have not heard yet.
pub(crate) fn take_notices(dobj: &mut AuthorityDObjState) -> Vec<Notice> {
    dobj.t5_destructible
        .as_mut()
        .map(|s| std::mem::take(&mut s.notices))
        .unwrap_or_default()
}

#[derive(Clone)]
struct Break {
    piece: usize,
    stage: usize,
}

/// What one hit did: the stages left, the stages that break on their own
/// later (piece, seconds, damage), whether the base piece died.
#[derive(Default)]
struct Outcome {
    breaks: Vec<Break>,
    after: Vec<(usize, f32, i32)>,
    died: bool,
    base_loop: Option<Option<String>>,
}

impl State {
    /// BO2's `DamagePiece`; `children`: the pieces under this one take their
    /// share (false only for fire, and only for the piece it struck).
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        index: usize,
        amount: i32,
        means: u8,
        children: bool,
        exclude: Option<usize>,
        depth: u32,
        out: &mut Outcome,
    ) {
        if depth > 20 {
            diag::warn!(
                Sim,
                "T5 DamagePiece recursion limit: {}",
                self.definition.name
            );
            return;
        }
        let piece = &self.definition.pieces[index];
        let old = piece.stage(index, self.health[index]);
        let mut health = self.health[index]
            .wrapping_sub(amount.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
            .max(-1);
        let mut next = piece.stage(index, health);
        let previous = old.map(|s| &piece.stages[s]);
        // bo2mp: BO2's DamagePiece: a knife breaks only a stage that lets it
        // (flag 8).
        if self.hosted && means == T6_MOD_MELEE && previous.is_some_and(|s| s.flags & 8 == 0) {
            return;
        }
        // bo2mp: a blast's splash goes through a stage that holds against
        // the rest (flag 1).
        let splash = self.hosted
            && matches!(
                means,
                T6_MOD_GRENADE_SPLASH | T6_MOD_PROJECTILE_SPLASH | T6_MOD_EXPLOSIVE
            );
        if !splash {
            if previous.is_some_and(|s| s.flags & 1 != 0) {
                return;
            }
            // bo2mp: BO2 numbers a destroyed piece's stage 5 and walks up to
            // that one too.
            let end = match next {
                Some(end) => Some(end),
                None if self.hosted => old.map(|_| piece.stages.len()),
                None => None,
            };
            if let Some(end) = end {
                for j in old.map_or(0, |s| s + 1)..=end {
                    // Stage 5 is one past BO2's five (types.h): its
                    // breakHealth is the piece's parentDamagePercent and its
                    // flags are its explosiveDamageScale's bits.
                    let (flags, break_health) = piece.stages.get(j).map_or(
                        (
                            piece.explosive_damage_scale.to_bits(),
                            piece.parent_damage_percent,
                        ),
                        |s| (s.flags, s.break_health),
                    );
                    if flags & 1 != 0 {
                        health = ((piece.health as f32 * break_health) as i16).wrapping_add(1);
                        next = piece.stage(index, health);
                        break;
                    }
                }
            }
        }
        let parent = usize::from(piece.parent_piece);
        if old != next
            && previous.is_some_and(|s| s.has_phys_preset && s.flags & 4 != 0)
            && self.health.get(parent).is_some_and(|h| *h > 0)
        {
            return;
        }
        let damage_parent = previous.is_some_and(|s| s.flags & 2 != 0);
        if index == 0 && self.health[index] > 0 && health <= 0 {
            out.died = true;
        }
        self.health[index] = health;
        if old != next {
            // The base piece's stage plays its loop on the entity (a car's fire).
            if index == 0 {
                out.base_loop = Some(next.and_then(|n| piece.stages[n].loop_sound.clone()));
            }
            if let Some(start) = old {
                for j in start..next.unwrap_or(5) {
                    if piece.stages[j].show_bone.is_some() {
                        out.breaks.push(Break {
                            piece: index,
                            stage: j,
                        });
                    }
                }
            }
            let after = next.map_or(0.0, |next| piece.stages[next].max_time);
            if after > 0.0 && self.hosted {
                out.after.push((index, after, piece.health));
            } else if after > 0.0 {
                diag::warn!(
                    Sim,
                    "T5 destructible breakafter callback unhosted: {} piece={}",
                    self.definition.name,
                    index
                );
            }
        }
        for child in (0..self.health.len()).filter(|_| children) {
            let p = &self.definition.pieces[child];
            if usize::from(p.parent_piece) == index && Some(index) != exclude {
                let damage = (amount as f32 * p.parent_damage_percent) as i32;
                if damage != 0 {
                    self.damage(child, damage, means, true, exclude, depth + 1, out);
                }
            }
        }
        if damage_parent && parent < self.health.len() {
            self.damage(parent, amount, means, true, Some(index), depth + 1, out);
        }
    }
}

fn update_hide_parts(dobj: &mut AuthorityDObjState) {
    let Some(state) = &dobj.t5_destructible else {
        return;
    };
    let Some(cap) = &dobj.capability else {
        return;
    };
    let hide = state
        .definition
        .hide_parts(&state.health, &cap.pose.bone_names);
    if hide != dobj.semantic_state.hide_part_bits {
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            diag::info!(
                Sim,
                "bo2mp destructible {} hides {:08x?} (bones {:?}, health {:?})",
                state.definition.name,
                hide.words(),
                cap.pose.bone_names,
                state.health
            );
        }
        dobj.semantic_state.hide_part_bits = hide;
        dobj.pose_request.hide_part_bits = hide;
        dobj.pose_revision = dobj.pose_revision.wrapping_add(1);
        dobj.semantic_state.pose_revision = dobj.pose_revision;
        dobj.current_collision = None;
        dobj.materialized_pose_revision = None;
        // bo2mp: at once, as BO2's DObj takes the new hide bits: the next
        // bullet this tick must miss the part that just broke off, not find
        // no shape at all and stop on the car's walk brushes.
        dobj.materialize();
    }
}

/// bo2mp: Black Ops II's push on a loose piece per unit of hit
/// (`dynEnt_bulletForce`, `dynEnt_explodeForce`) and a blast's lift
/// (`dynEnt_explodeUpbias`), and the least push a blast gives
/// (`dynEnt_explodeMinForce`). The push is an impulse: speed = push / mass.
const BULLET_FORCE: f32 = 0.8;
const EXPLODE_FORCE: f32 = 12.0;
const EXPLODE_UPBIAS: f32 = 0.5;
const EXPLODE_MIN_FORCE: f32 = 0.01;

/// bo2mp: how fast a piece leaves when a hit going `dir` breaks it off:
/// the engine's push scaled by the piece's preset, over its mass. A
/// script's damage (no direction) lets it drop.
fn launch_velocity(
    preset: &xmodel_runtime::DebrisPreset,
    kind: DamageKind,
    dir: Option<[f32; 3]>,
) -> [f32; 3] {
    let raw = dir.map(glam::Vec3::from_array);
    let Some(dir) = raw.and_then(glam::Vec3::try_normalize) else {
        return [0.0; 3];
    };
    let push = match kind {
        DamageKind::Explosive | DamageKind::Blast | DamageKind::Splash => {
            // A blast's direction is as long as its falloff (apply_blast_by).
            let falloff = raw.map_or(1.0, |r| r.length().min(1.0));
            let force = EXPLODE_FORCE * preset.explosive_force_scale * falloff;
            if force <= EXPLODE_MIN_FORCE {
                return [0.0; 3];
            }
            let lifted = (dir + glam::Vec3::Z * EXPLODE_UPBIAS)
                .try_normalize()
                .unwrap_or(dir);
            lifted * force
        }
        DamageKind::Bullet | DamageKind::Melee | DamageKind::Impact => {
            dir * BULLET_FORCE * preset.bullet_force_scale
        }
        DamageKind::Burned | DamageKind::Script => glam::Vec3::ZERO,
    };
    (push / preset.mass.max(1e-4)).to_array()
}

#[allow(clippy::too_many_arguments)]
fn publish(
    world: &mut FrameWorld,
    tick: Tick,
    owner: AuthorityModelOwner,
    out: Outcome,
    kind: DamageKind,
    dir: Option<[f32; 3]>,
    attacker: Option<ClientId>,
    weapon: u32,
) {
    let Outcome {
        breaks,
        after,
        died,
        base_loop,
    } = out;
    if breaks.is_empty() && after.is_empty() && !died && base_loop.is_none() {
        return;
    }
    let Some(dobj) = world
        .entity_collision_capabilities_mut()
        .iter_mut()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_mut())
    else {
        return;
    };
    let Some(state) = &mut dobj.t5_destructible else {
        return;
    };
    let definition = state.definition.clone();
    let hosted = state.hosted;
    if hosted {
        for (piece, time, damage) in after {
            state.notices.push(Notice::BreakAfter {
                piece,
                time,
                damage,
            });
        }
    }
    let health = state.health.clone();
    let mut notices = Vec::new();
    let pose = dobj
        .capability
        .as_ref()
        .and_then(|cap| cap.pose(&dobj.pose_request, dobj.world_from_model).ok());
    let mut events = Vec::new();
    let mut debris = Vec::new();
    for event in breaks {
        let piece = &definition.pieces[event.piece];
        let st = &piece.stages[event.stage];

        let bone = dobj.capability.as_ref().and_then(|cap| {
            cap.pose
                .bone_names
                .iter()
                .position(|name| Some(name) == piece.stages[0].show_bone.as_ref())
        });
        let matrix = bone
            .and_then(|b| pose.as_ref()?.get(b))
            .copied()
            .unwrap_or(dobj.world_from_model);
        let origin = matrix.w_axis.truncate().to_array();
        let direction = matrix.x_axis.truncate().normalize().to_array();
        diag::info!(
            Sim,
            "T5 destructible {} piece={} leaving_stage={} fx={:?} health={}",
            definition.name,
            event.piece,
            event.stage,
            st.break_effect,
            health[event.piece]
        );
        // The pieces this stage drops come loose where its bone was.
        let shown = dobj.capability.as_ref().and_then(|cap| {
            cap.pose
                .bone_names
                .iter()
                .position(|name| Some(name) == st.show_bone.as_ref())
        });
        let (_, turn, at) = shown
            .and_then(|b| pose.as_ref()?.get(b))
            .copied()
            .unwrap_or(matrix)
            .to_scale_rotation_translation();
        for (model, preset) in st.spawn_models.iter().zip(&st.spawn_presets) {
            let (Some(model), Some(preset)) = (model, preset) else {
                continue;
            };
            let velocity = launch_velocity(preset, kind, dir);
            if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
                diag::info!(
                    Sim,
                    "bo2mp destructible {} drops {model} at {at:?} flying {velocity:?}",
                    definition.name
                );
            }
            debris.push(crate::DebrisRecord {
                model: model.clone(),
                origin: at.to_array(),
                rotation: turn.to_array(),
                velocity,
                mass: preset.mass,
                bounce: preset.bounce,
                friction: preset.friction,
                bullet_force_scale: preset.bullet_force_scale,
                explosive_force_scale: preset.explosive_force_scale,
            });
        }
        if let Some(notify) = &st.break_notify
            && hosted
        {
            notices.push(Notice::Broken {
                notify: notify.clone(),
                attacker,
                weapon,
            });
        } else if let Some(notify) = &st.break_notify {
            diag::warn!(
                Sim,
                "T5 destructible script notify unhosted: {} {}",
                definition.name,
                notify
            );
        }
        if let Some(fx) = &st.break_effect {
            events.push((
                entity_t6::EntityEventKind::PLAY_FX,
                fx.clone(),
                origin,
                direction,
            ));
        }
        if let Some(sound) = &st.break_sound {
            events.push((
                entity_t6::EntityEventKind::SOUND_ALIAS,
                sound.clone(),
                origin,
                direction,
            ));
        }
    }
    match base_loop {
        Some(alias) if hosted => notices.push(Notice::Loop(alias)),
        Some(Some(alias)) => diag::warn!(
            Sim,
            "T5 destructible loop sound unhosted: {} {}",
            definition.name,
            alias
        ),
        _ => {}
    }
    if hosted && died {
        notices.push(Notice::BaseDestroyed { attacker });
    }
    if let Some(state) = &mut dobj.t5_destructible {
        state.notices.extend(notices);
    }
    update_hide_parts(dobj);
    for record in debris {
        world.push_debris(record);
    }
    for (kind, name, origin, direction) in events {
        let index = if kind == entity_t6::EntityEventKind::PLAY_FX {
            world.effect_name_index(&name)
        } else {
            world.sound_alias_index(&name)
        };
        world.push_entity_event(
            tick,
            EventAudience::All,
            kind,
            crate::EntityEventPayload {
                number: i32::from(trace_t6::ENTITYNUM_WORLD),
                event_parm: i32::from(index),
                origin,
                direction,
                ..Default::default()
            },
        );
    }
}

/// A blast (a grenade, a script's `radiusdamage`) by `attacker`'s `weapon`
/// damages each standing piece of every breakable prop in reach, by the
/// distance from the piece's bone, times the piece's explosive scale.
/// Returns each prop it reached (a Black Ops II map's props queue their
/// notifies as notices, see [`publish`]). bo2mp: a Black Ops II map's prop
/// takes it the way BO2's G_RadiusDamage gives it: reached when the blast
/// is nearer than `radius` to its bounds and sees it ([`can_damage`];
/// `skip` are the blast's own models, its inflictor's, which it spares), its pieces by
/// [`radius_pieces`], then the entity's G_Damage (a [`Notice::Damage`] of
/// the amount at that distance). `means` is the blast's kind of death.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_blast_by(
    world: &mut FrameWorld,
    tick: Tick,
    origin: [f32; 3],
    radius: f32,
    amount_at: impl Fn(f32) -> f32,
    means: u8,
    skip: &[AuthorityModelOwner],
    attacker: Option<ClientId>,
    weapon: u32,
) -> Vec<(AuthorityModelOwner, Vec<String>)> {
    let origin = glam::Vec3::from_array(origin);
    let mut hits = Vec::new();
    let mut hosted = Vec::new();
    for row in world.entity_collision_capabilities() {
        let Some(dobj) = row.dobj.as_ref() else {
            continue;
        };
        let (Some(state), Some(cap)) = (&dobj.t5_destructible, &dobj.capability) else {
            continue;
        };
        if state.hosted {
            hosted.push((row.owner, dobj.world_from_model, cap.bounds));
            continue;
        }
        if dobj.world_from_model.w_axis.truncate().distance(origin) > radius + 128.0 {
            continue;
        }
        let Ok(pose) = cap.pose(&dobj.pose_request, dobj.world_from_model) else {
            continue;
        };
        for (i, piece) in state.definition.pieces.iter().enumerate() {
            let Some(stage) = piece.stage(i, state.health[i]) else {
                continue;
            };
            let at = piece.stages[stage]
                .show_bone
                .as_ref()
                .and_then(|name| cap.pose.bone_names.iter().position(|n| n == name))
                .and_then(|b| pose.get(b))
                .map_or(dobj.world_from_model.w_axis, |m| m.w_axis)
                .truncate();
            let d = at.distance(origin);
            if d > radius {
                continue;
            }
            // Not through walls (a wall prop's own wall stops the line just
            // short of it).
            let start = (origin + glam::Vec3::Z * 4.0).to_array();
            let t = world.trace_static_world(
                start,
                at.to_array(),
                [0.0; 3],
                [0.0; 3],
                crate::bullet_collision::MASK_SHOT,
            );
            if (1.0 - t.fraction) * d > 12.0 {
                continue;
            }
            let amount = (amount_at(d).round() * piece.explosive_damage_scale) as i32;
            if amount > 0 {
                // Black Ops II's push falls off in a line to nothing at the
                // edge (DynEntCl_ExplosionEvent); its length carries that.
                let falloff = (1.0 - d / radius.max(1.0)).clamp(0.0, 1.0);
                let dir = (at - origin).normalize_or(glam::Vec3::Z) * falloff;
                hits.push((row.owner, i, amount, dir.to_array()));
            }
        }
    }
    let mut out = Vec::new();
    for (owner, index, amount, dir) in hits {
        let Some(dobj) = world
            .entity_collision_capabilities_mut()
            .iter_mut()
            .find(|r| r.owner == owner)
            .and_then(|r| r.dobj.as_mut())
        else {
            continue;
        };
        let Some(state) = &mut dobj.t5_destructible else {
            continue;
        };
        let mut hit = Outcome::default();
        state.damage(index, amount, means, true, None, 0, &mut hit);
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            diag::info!(
                Sim,
                "destructible blast {}: piece {index} -{amount} -> health {:?} breaks {}",
                state.definition.name,
                state.health,
                hit.breaks.len()
            );
        }
        // (bo2mp: an unhosted prop's notifies are logged by `publish`.)
        publish(
            world,
            tick,
            owner,
            hit,
            DamageKind::Explosive,
            Some(dir),
            attacker,
            weapon,
        );
        out.push((owner, Vec::new()));
    }
    let mut reached = Vec::new();
    for (owner, world_from_model, bounds) in hosted {
        // BO2's G_RadiusDamage leaves out its `ignore`: the entity whose
        // own `radiusdamage` it is (a car's blast spares the car).
        if skip.contains(&owner) {
            continue;
        }
        let at = world_from_model.w_axis.truncate();
        // BO2 measures to the nearest point of the model's bounds.
        let d = match bounds {
            Some((mid, half)) => {
                let (mid, half) = (glam::Vec3::from_array(mid), glam::Vec3::from_array(half));
                let local = world_from_model.inverse().transform_point3(origin);
                local.distance(local.clamp(mid - half, mid + half))
            }
            None => at.distance(origin),
        };
        if d >= radius {
            continue;
        }
        let mut ignore = vec![owner];
        ignore.extend_from_slice(skip);
        if !can_damage(world, origin, world_from_model, bounds, &ignore) {
            if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
                diag::info!(
                    Sim,
                    "destructible blast at {origin} hidden from {owner:?} ({d} away)"
                );
            }
            continue;
        }
        reached.push((owner, d, at));
    }
    for (owner, d, at) in reached {
        radius_pieces(world, tick, owner, origin, radius, &amount_at, means, attacker, weapon);
        push_notice(
            world,
            owner,
            Notice::Damage {
                amount: (amount_at(d) as i32).max(1),
                attacker,
                means,
                weapon,
                point: origin.to_array(),
                dir: (at - origin + glam::Vec3::Z * 24.0)
                    .try_normalize()
                    .map(|v| v.to_array()),
                flags: 5,
            },
        );
        out.push((owner, Vec::new()));
    }
    out
}

/// bo2mp: BO2's CanDamage on a prop: whether a blast at `origin` sees the
/// middle of its bounds or one of four points around it, square to the
/// line from the blast (a trace that leaves out `skip`).
pub(crate) fn can_damage(
    world: &FrameWorld,
    origin: glam::Vec3,
    world_from_model: glam::Mat4,
    bounds: Option<([f32; 3], [f32; 3])>,
    skip: &[AuthorityModelOwner],
) -> bool {
    let (mid, half) = bounds.map_or((glam::Vec3::ZERO, glam::Vec3::ZERO), |(m, h)| {
        (glam::Vec3::from_array(m), glam::Vec3::from_array(h))
    });
    let c = world_from_model.w_axis.truncate() + mid;
    let v = origin - c;
    let len = v.length();
    let mut dir = if len > 0.0 { v / len } else { glam::Vec3::ZERO };
    if dir.x * dir.x + dir.y * dir.y < 0.001 {
        dir = glam::Vec3::X;
    }
    let side = glam::Vec3::new(-dir.y, dir.x, 0.0).normalize_or_zero();
    let up = dir.cross(side);
    let s = side * ((half.x * side.x).abs() + (half.y * side.y).abs());
    let u = up * ((up.x * half.x).abs() + (up.y * half.y).abs() + (up.z * half.z).abs());
    [c, c + s + u, c - s + u, c + s - u, c - s - u].iter().any(|p| {
        world
            .trace_world_skipping(origin.to_array(), p.to_array(), 0x0080_2013, skip)
            .fraction
            >= 1.0
    })
}

/// bo2mp: BO2's DestructibleRadiusDamage on one prop: each piece in turn (a
/// later piece sees what the earlier ones did to it) takes the blast's
/// amount at its distance from `origin` times its explosive scale, from
/// where [`piece_point`] puts it (whether the blast sees the prop at all is
/// the caller's: CanDamage).
#[allow(clippy::too_many_arguments)]
fn radius_pieces(
    world: &mut FrameWorld,
    tick: Tick,
    owner: AuthorityModelOwner,
    origin: glam::Vec3,
    radius: f32,
    amount_at: &dyn Fn(f32) -> f32,
    means: u8,
    attacker: Option<ClientId>,
    weapon: u32,
) {
    let found = world
        .entity_collision_capabilities()
        .iter()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_ref())
        .and_then(|d| {
            let cap = d.capability.clone()?;
            let pose = cap.pose(&d.pose_request, d.world_from_model).ok()?;
            let pieces = d.t5_destructible.as_ref()?.health.len();
            Some((cap, pose, d.world_from_model, pieces))
        });
    let Some((cap, pose, world_from_model, pieces)) = found else {
        return;
    };
    for index in 0..pieces {
        let Some((at, scale)) = world
            .entity_collision_capabilities()
            .iter()
            .find(|r| r.owner == owner)
            .and_then(|r| r.dobj.as_ref()?.t5_destructible.as_ref())
            .and_then(|state| {
                let at = piece_point(state, &cap, &pose, world_from_model, index)?;
                Some((at, state.definition.pieces[index].explosive_damage_scale))
            })
        else {
            continue;
        };
        let d = at.distance(origin);
        if d >= radius {
            continue;
        }
        let amount = (amount_at(d) * scale) as i32;
        if amount == 0 {
            continue;
        }
        let Some(state) = world
            .entity_collision_capabilities_mut()
            .iter_mut()
            .find(|r| r.owner == owner)
            .and_then(|r| r.dobj.as_mut()?.t5_destructible.as_mut())
        else {
            return;
        };
        let mut hit = Outcome::default();
        state.damage(index, amount, means, true, None, 0, &mut hit);
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            diag::info!(
                Sim,
                "destructible blast {}: piece {index} -{amount} (mod {means}) -> health {:?} breaks {}",
                state.definition.name,
                state.health,
                hit.breaks.len()
            );
        }
        // Black Ops II's push falls off in a line to nothing at the edge
        // (DynEntCl_ExplosionEvent); its length carries that.
        let falloff = (1.0 - d / radius.max(1.0)).clamp(0.0, 1.0);
        let dir = (if d > 0.001 {
            (at - origin) / d
        } else {
            glam::Vec3::Z
        }) * falloff;
        publish(
            world,
            tick,
            owner,
            hit,
            DamageKind::Explosive,
            Some(dir.to_array()),
            attacker,
            weapon,
        );
    }
}

/// bo2mp: where BO2's DestructibleRadiusDamage measures a piece from: the
/// middle of its shown bone's bounds. A destroyed piece shows no bone on the
/// server (GetShowBoneNameForPiece), so it counts from the prop's origin, as
/// does a bone the model lacks. None skips the piece: a child piece's front,
/// right or left effect bone, or a bone with no place.
fn piece_point(
    state: &State,
    cap: &RetainedModelCapability,
    pose: &[glam::Mat4],
    world_from_model: glam::Mat4,
    index: usize,
) -> Option<glam::Vec3> {
    let piece = &state.definition.pieces[index];
    let shown = piece
        .stage(index, state.health[index])
        .and_then(|s| piece.stages[s].show_bone.as_deref());
    if usize::from(piece.parent_piece) < state.health.len()
        && shown.is_some_and(|b| matches!(b, "tag_fx_front" | "tag_fx_right" | "tag_fx_left"))
    {
        return None;
    }
    let Some(b) = shown.and_then(|name| cap.pose.bone_names.iter().position(|n| n == name)) else {
        return Some(world_from_model.w_axis.truncate());
    };
    let middle = cap
        .bone_collision
        .get(b)
        .and_then(|c| c.as_ref())
        .map_or([0.0; 3], |c| c.midpoint);
    Some(pose.get(b)?.transform_point3(glam::Vec3::from_array(middle)))
}

/// bo2mp: the scale BO2's DestructibleDamage counts a hit on `piece` by, from
/// its means of death.
fn means_scale(piece: &T5DestructiblePiece, means: u8) -> f32 {
    match means {
        MOD_GRENADE | MOD_PROJECTILE | T6_MOD_EXPLOSIVE => piece.explosive_damage_scale,
        T6_MOD_MELEE => piece.melee_damage_scale,
        T6_MOD_IMPACT => piece.impact_damage_scale,
        _ => piece.bullet_damage_scale,
    }
}

/// bo2mp: a script's `dodamage` on a Black Ops II map's prop, the way BO2's
/// ScrCmd_DoDamage gives it: first a blast from `point`
/// (DestructibleRadiusDamage: `amount` close in, 10 at 400 units, by
/// `means`, or MOD_EXPLOSIVE for none), then the hit itself (G_Damage with
/// `flags` and 0x10, pointed from `point` to the prop, see
/// [`apply_damage`]). False when `owner` is not a destructible.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_script_damage(
    world: &mut FrameWorld,
    tick: Tick,
    owner: AuthorityModelOwner,
    point: [f32; 3],
    amount: f32,
    means: u8,
    kind: DamageKind,
    flags: i32,
    attacker: Option<ClientId>,
    weapon: u32,
) -> bool {
    let Some((hosted, origin)) = world
        .entity_collision_capabilities()
        .iter()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_ref())
        .and_then(|d| {
            let hosted = d.t5_destructible.as_ref()?.hosted;
            Some((hosted, d.world_from_model.w_axis.truncate()))
        })
    else {
        return false;
    };
    let mut dir = None;
    if hosted {
        let blast = if means == T6_MOD_UNKNOWN {
            T6_MOD_EXPLOSIVE
        } else {
            means
        };
        radius_pieces(
            world,
            tick,
            owner,
            glam::Vec3::from_array(point),
            400.0,
            &|d| (1.0 - d / 400.0) * (amount - 10.0) + 10.0,
            blast,
            attacker,
            weapon,
        );
        dir = (origin - glam::Vec3::from_array(point))
            .try_normalize()
            .map(|v| v.to_array());
    }
    apply_damage(
        world,
        tick,
        owner,
        None,
        amount as i32,
        kind,
        means,
        point,
        dir,
        flags | 0x10,
        attacker,
        weapon,
    )
}

/// A bullet (by `means`) striking `bone` of a breakable at `point`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_hit(
    world: &mut FrameWorld,
    tick: Tick,
    owner: AuthorityModelOwner,
    bone: u16,
    amount: u32,
    means: u8,
    point: [f32; 3],
    dir: [f32; 3],
    attacker: Option<ClientId>,
    weapon: u32,
) -> bool {
    let amount = i32::try_from(amount).unwrap_or(i32::MAX);
    apply_damage(
        world,
        tick,
        owner,
        Some(bone),
        amount,
        DamageKind::Bullet,
        means,
        point,
        Some(dir),
        0,
        attacker,
        weapon,
    )
}

/// bo2mp: damage a destructible: the piece whose shown bone (or a bone
/// under it) was struck, else the base piece, by that piece's scale for the
/// kind. A piece it breaks off flies along `dir`. False when `owner` is
/// not a destructible. bo2mp: a Black Ops II map's prop takes it the way
/// BO2's G_Damage and DestructibleDamage give it: at least 1, by `means`,
/// and with no bone, on the first destroyed piece; a blast's share (`flags`
/// & 1) leaves the pieces alone. Then the entity's own health takes what
/// DestructibleDamage hands back (a [`Notice::Damage`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_damage(
    world: &mut FrameWorld,
    tick: Tick,
    owner: AuthorityModelOwner,
    bone: Option<u16>,
    amount: i32,
    kind: DamageKind,
    means: u8,
    point: [f32; 3],
    dir: Option<[f32; 3]>,
    flags: i32,
    attacker: Option<ClientId>,
    weapon: u32,
) -> bool {
    // BO2's DestructibleDamage: a burn on a Black Ops II map's prop takes a
    // random 1..=amount (G_rand) off every piece.
    let pieces_burned = world
        .entity_collision_capabilities()
        .iter()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_ref()?.t5_destructible.as_ref())
        .filter(|s| s.hosted && means == MOD_BURNED)
        .map(|s| s.health.len());
    let burns: Vec<i32> = pieces_burned.map_or_else(Vec::new, |pieces| {
        let rng = world.combat_rng_mut();
        (0..pieces)
            .map(|_| (rng.next_u32() % amount.max(1) as u32) as i32 + 1)
            .collect()
    });
    let Some(dobj) = world
        .entity_collision_capabilities_mut()
        .iter_mut()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_mut())
    else {
        return false;
    };
    let Some(state) = &mut dobj.t5_destructible else {
        return false;
    };
    let index = match bone {
        // bo2mp: BO2 asks for the piece that shows no bone: the first
        // destroyed one (the server shows none for it), else the base.
        None if state.hosted => (0..state.health.len())
            .find(|&i| state.definition.pieces[i].stage(i, state.health[i]).is_none())
            .unwrap_or(0),
        _ => bone
            .zip(dobj.capability.as_ref())
            .and_then(|(bone, cap)| struck_piece(state, &cap.pose, usize::from(bone)))
            .unwrap_or(0),
    };
    let hosted = state.hosted;
    let piece = &state.definition.pieces[index];
    let transfer = piece.entity_damage_transfer;
    let (amount, scale) = if hosted {
        (amount.max(1), means_scale(piece, means))
    } else {
        let scale = match kind {
            DamageKind::Bullet | DamageKind::Splash => piece.bullet_damage_scale,
            DamageKind::Explosive | DamageKind::Blast => piece.explosive_damage_scale,
            DamageKind::Melee => piece.melee_damage_scale,
            DamageKind::Impact => piece.impact_damage_scale,
            DamageKind::Burned | DamageKind::Script => 1.0,
        };
        (amount, scale)
    };
    let damage = (amount as f32 * scale) as i32;
    // BO2's G_Damage skips DestructibleDamage for a blast's share (dflags &
    // 1): DestructibleRadiusDamage gave the pieces theirs.
    let pieces = !(hosted && flags & 1 != 0);
    if pieces {
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            let tag = bone
                .zip(dobj.capability.as_ref())
                .and_then(|(b, cap)| cap.pose.bone_names.get(usize::from(b)).cloned());
            let shows: Vec<_> = piece.stages.iter().map(|s| s.show_bone.clone()).collect();
            diag::info!(
                Sim,
                "bo2mp destructible {} hit on {tag:?}: piece {index} of {} (stages show {shows:?}) \
                 takes {damage} ({kind:?} x{scale}), health {:?}",
                state.definition.name,
                state.definition.pieces.len(),
                state.health
            );
        }
        let mut out = Outcome::default();
        if burns.is_empty() {
            // BO2's DestructibleDamage spares the children of piece 5.
            let exclude = hosted.then_some(5);
            state.damage(index, damage, means, true, exclude, 0, &mut out);
        } else {
            // Unscaled, and none passed to the pieces under each.
            for (index, burn) in burns.into_iter().enumerate() {
                state.damage(index, burn, means, false, None, 0, &mut out);
            }
        }
        publish(world, tick, owner, out, kind, dir, attacker, weapon);
    }
    if hosted {
        // What the entity's own health loses: the whole hit for a blast's
        // share, a script's dodamage (0x10) or a burn; nothing for a knife;
        // else the piece's share of what it took (entityDamageTransfer).
        let taken = if !pieces || flags & 0x10 != 0 || means == MOD_BURNED {
            amount
        } else if means == T6_MOD_MELEE {
            0
        } else if transfer > 0.0 {
            (damage as f32 * transfer) as i32
        } else {
            0
        };
        push_notice(
            world,
            owner,
            Notice::Damage {
                amount: taken,
                attacker,
                means,
                weapon,
                point,
                dir,
                flags,
            },
        );
    }
    true
}

/// bo2mp: queue `notice` on a Black Ops II map's prop (t6/destructible.rs
/// hands it to the scripts).
fn push_notice(world: &mut FrameWorld, owner: AuthorityModelOwner, notice: Notice) {
    if let Some(state) = world
        .entity_collision_capabilities_mut()
        .iter_mut()
        .find(|r| r.owner == owner)
        .and_then(|r| r.dobj.as_mut()?.t5_destructible.as_mut())
    {
        state.notices.push(notice);
    }
}

/// The piece showing the struck bone, or the nearest bone above it that a
/// piece shows. bo2mp: a Black Ops II map's prop takes the struck bone
/// exactly (BO2's `Destructible_GetPieceIndexFromBoneName`; a bone under it
/// is the base piece's).
fn struck_piece(state: &State, pose: &xmodel_runtime::ModelPoseSrc, bone: usize) -> Option<usize> {
    let mut b = bone;
    loop {
        let tag = pose.bone_names.get(b)?;
        let found = state.definition.pieces.iter().enumerate().position(|(i, p)| {
            p.stage(i, state.health[i])
                .is_some_and(|st| p.stages[st].show_bone.as_ref() == Some(tag))
        });
        if found.is_some() || state.hosted {
            return found;
        }
        let step = usize::from(*pose.parent_list.get(b.checked_sub(pose.num_root_bones)?)?);
        b = b.checked_sub(step).filter(|_| step > 0)?;
    }
}

#[cfg(test)]
mod tests {
    use super::{MOD_BURNED, MOD_GRENADE, Outcome, State};
    use crate::world_objects::{
        T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH, T6_MOD_IMPACT, T6_MOD_MELEE, T6_MOD_RIFLE_BULLET,
    };
    use std::sync::Arc;
    use xmodel_runtime::{T5DestructibleDef, T5DestructiblePiece, T5DestructibleStage};

    /// A stage showing `bone` from `break_health` of the piece's health down.
    fn stage(bone: &str, break_health: f32, flags: u32) -> T5DestructibleStage {
        T5DestructibleStage {
            show_bone: Some(bone.to_owned()),
            break_health,
            flags,
            ..Default::default()
        }
    }

    fn piece(stages: Vec<T5DestructibleStage>, parent: u8, share: f32) -> T5DestructiblePiece {
        let mut all: [T5DestructibleStage; 5] = Default::default();
        for (slot, st) in all.iter_mut().zip(stages) {
            *slot = st;
        }
        T5DestructiblePiece {
            stages: all,
            parent_piece: parent,
            parent_damage_percent: share,
            bullet_damage_scale: 1.0,
            explosive_damage_scale: 1.0,
            melee_damage_scale: 1.0,
            impact_damage_scale: 1.0,
            entity_damage_transfer: 0.0,
            health: 100,
            hide_bones: [0; 5],
        }
    }

    fn state(pieces: Vec<T5DestructiblePiece>) -> State {
        State {
            health: pieces.iter().map(|p| p.health as i16).collect(),
            definition: Arc::new(T5DestructibleDef {
                name: "test".to_owned(),
                model: "test".to_owned(),
                pieces,
                client_only: false,
            }),
            hosted: true,
            notices: Vec::new(),
        }
    }

    /// One base piece: whole down to half, then broken.
    fn base(flags: u32) -> State {
        state(vec![piece(
            vec![stage("body", 0.5, flags), stage("body_d1", 0.0, 0)],
            255,
            0.0,
        )])
    }

    fn hit(s: &mut State, index: usize, amount: i32, means: u8, children: bool) -> Outcome {
        let mut out = Outcome::default();
        s.damage(index, amount, means, children, None, 0, &mut out);
        out
    }

    #[test]
    fn a_knife_breaks_only_a_stage_that_allows_it() {
        let mut s = base(0);
        hit(&mut s, 0, 60, T6_MOD_MELEE, true);
        assert_eq!(s.health, [100]);
        let mut s = base(8);
        hit(&mut s, 0, 60, T6_MOD_MELEE, true);
        assert_eq!(s.health, [40]);
    }

    #[test]
    fn a_holding_stage_stops_all_but_a_blast() {
        for means in [T6_MOD_RIFLE_BULLET, MOD_GRENADE, T6_MOD_IMPACT] {
            let mut s = base(1);
            hit(&mut s, 0, 60, means, true);
            assert_eq!(s.health, [100], "means {means}");
        }
        for means in [T6_MOD_EXPLOSIVE, T6_MOD_GRENADE_SPLASH] {
            let mut s = base(1);
            hit(&mut s, 0, 60, means, true);
            assert_eq!(s.health, [40], "means {means}");
        }
    }

    #[test]
    fn a_holding_stage_catches_a_hit_that_would_destroy_the_piece() {
        // A door: whole, then holding from a fifth of its health, then hanging.
        let mut s = state(vec![
            piece(vec![stage("body", 0.0, 0)], 255, 0.0),
            piece(
                vec![
                    stage("door", 0.5, 0),
                    stage("door_d1", 0.2, 1),
                    stage("door_d2", 0.0, 0),
                ],
                0,
                0.0,
            ),
        ]);
        hit(&mut s, 1, 500, T6_MOD_RIFLE_BULLET, true);
        assert_eq!(s.health[1], 21, "held just inside the holding stage");
        hit(&mut s, 1, 500, T6_MOD_RIFLE_BULLET, true);
        assert_eq!(s.health[1], 21, "a bullet cannot break past it");
        hit(&mut s, 1, 500, T6_MOD_EXPLOSIVE, true);
        assert_eq!(s.health[1], -1, "a blast destroys it");
    }

    #[test]
    fn fire_spares_the_pieces_under_the_one_it_burns() {
        let pieces = || {
            vec![
                piece(vec![stage("body", 0.0, 0)], 255, 0.0),
                piece(vec![stage("hood", 0.0, 0)], 0, 0.5),
            ]
        };
        let mut s = state(pieces());
        hit(&mut s, 0, 40, MOD_BURNED, false);
        assert_eq!(s.health, [60, 100]);
        let mut s = state(pieces());
        hit(&mut s, 0, 40, T6_MOD_RIFLE_BULLET, true);
        assert_eq!(s.health, [60, 80]);
    }

    #[test]
    fn the_base_pieces_new_stage_plays_its_loop() {
        let mut s = base(0);
        Arc::make_mut(&mut s.definition).pieces[0].stages[1].loop_sound = Some("fire".to_owned());
        let out = hit(&mut s, 0, 60, T6_MOD_RIFLE_BULLET, true);
        assert_eq!(out.base_loop, Some(Some("fire".to_owned())));
        assert!(!out.died);
        // Destroyed, the base piece keeps its last stage: no new loop, a death.
        let out = hit(&mut s, 0, 100, T6_MOD_RIFLE_BULLET, true);
        assert_eq!(out.base_loop, None);
        assert!(out.died);
        assert_eq!(s.health, [-1]);
    }
}
