//! bo2 gun brain: a deliberate departure from Black Ops II's own animation data.
//!
//! BO2's knife run loops (the combat knife, the ballistic knife) do not end where they start.
//! The ballistic knife's right arm swings back and then forward over the first half of its
//! loop, holds still for the second half, then jumps back in one frame as the loop wraps:
//! played the way BO2 plays it, the right hand stops mid-run every stride. Its left arm is the
//! mirror: still for the first half, back and forward over the second, then the same jump.
//! The pistols' and dual pistols' loops also wrap with a big step, but no bigger than their
//! own fast swing mid-loop (measured 10-20: wrap 52-93 degrees against 39-127 mid-loop; the
//! knives' 87-102 against 13-17): those play exactly as BO2 has them.
//! Where a run loop's wrap jumps like that, each arm keeps BO2's own swing and, over the half
//! it would hold still in, eases from where its swing ends back to where it starts: one
//! swing back per stride for each arm, the two arms half a stride apart, no hold and no jump.
//! (10-20: playing each arm's swing backward instead swung both arms back twice a stride, at
//! the same moments, so the arms rocked together.)
//! What a hand holds (the gun's bone `tag_weapon`, the gun's own root `tag_origin`) is not on
//! the hand's bones: it hangs off the view, like the arms, and BO2 keys it to follow the hand.
//! Given the view's skeleton, each held thing goes with the wrist BO2's frames keep it
//! steadiest against (the right wrist for the gun bone; the left for the ballistic knife,
//! which sits in the left fist while it hangs off the right arm's gun bone), and is set each
//! frame so it sits in that hand exactly as BO2 has it, eased in step with that arm.
//! Without the skeleton (the dual-wield left gun, the clip tools) each arm plays its swing back
//! the other way over its still half, the gun with the right arm.
//! Never a belt's bullets or a strap; a loop that already ends where it starts (every
//! rifle's) is left exactly as BO2 has it.
//!
//! Only `xmodel_runtime` types are used here, so the clip tools can include this file as is.

use xmodel_runtime::{AnimClip, FrameIndices, Keyed, Rotation, Track, Translation};

type Quat = [f32; 4];
type Vec3 = [f32; 3];
/// A bone's place: its turn and its offset.
type Place = (Quat, Vec3);

/// The viewmodel's combined skeleton (arms, gun, its parts) as the view poses it.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewSkeleton {
    bones: Vec<SkelBone>,
    /// Bones that copy an earlier bone of the same name each frame (copy, source).
    duplicates: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
struct SkelBone {
    name: String,
    parent: Option<usize>,
    bind: Place,
    /// How much a keyed move counts (the model's scale).
    scale: f32,
}

/// The skeleton of a combined viewmodel.
#[must_use]
pub fn view_skeleton(dobj: &xmodel_runtime::DObj) -> ViewSkeleton {
    let bones = dobj
        .bones
        .iter()
        .map(|b| {
            let (q, t) = (b.bind_rotation.to_array(), b.bind_translation.to_array());
            let scale = if b.no_scale { 1.0 } else { dobj.models.get(b.model).map_or(1.0, |m| m.scale) };
            SkelBone { name: b.name.clone(), parent: b.parent, bind: (q, t), scale }
        })
        .collect();
    ViewSkeleton { bones, duplicates: dobj.duplicates.clone() }
}

impl ViewSkeleton {
    fn find(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// The bone each of a clip's tracks moves: the first of its name, as the view binds them.
    fn tracks_for(&self, clip: &AnimClip) -> Vec<Option<usize>> {
        clip.tracks.iter().map(|t| self.find(&t.name)).collect()
    }

    /// Every bone's place in the view at whole loop frame `f`, posed the way the view poses one
    /// clip: a keyed bone takes the clip's turn (none when the track has no turn) and its bind
    /// offset plus the clip's move, scaled with its model; the rest keep their bind turn.
    fn pose(&self, clip: &AnimClip, tracks: &[Option<usize>], f: usize) -> Vec<Place> {
        let mut locals: Vec<Place> = self.bones.iter().map(|b| (b.bind.0, [0.0; 3])).collect();
        for (track, bone) in clip.tracks.iter().zip(tracks) {
            if let Some(bone) = *bone {
                locals[bone] = (turn_at(&track.rotation, f), move_at(&track.translation, f));
            }
        }
        for &(copy, source) in &self.duplicates {
            if let Some(&local) = locals.get(source) {
                locals[copy] = local;
            }
        }
        let mut world: Vec<Place> = Vec::with_capacity(self.bones.len());
        for (b, (q, t)) in self.bones.iter().zip(locals) {
            let local = (q, add(b.bind.1, t.map(|c| c * b.scale)));
            let placed = b.parent.and_then(|p| world.get(p).copied()).map_or(local, |w| mul_rt(w, local));
            world.push(placed);
        }
        world
    }
}

/// A run loop is smoothed when one bone's wrap turns it more than this (degrees). BO2's rifle
/// run loops wrap within a few degrees; its one-handed ones jump 40 to 130.
pub const CLIP_JUMP_DEG: f32 = 20.0;
/// Within a smoothed loop, a bone's wrap counts as a jump past this turn (degrees) or move
/// (inches), when it is also several times the bone's usual step in one frame.
const BONE_JUMP_DEG: f32 = 8.0;
const BONE_JUMP_UNITS: f32 = 0.5;
const USUAL_STEP_TIMES: f32 = 3.0;
/// A run loop is smoothed only when its worst wrap turns more than this many times the
/// biggest turn any bone makes in one frame inside the loop (the knives: 6 to 7; the pistols
/// 1.3, the dual pistols under 1).
pub const STANDOUT_TIMES: f32 = 3.0;
/// A held thing goes with a wrist it turns no more than 10 degrees per this, and moves no more
/// than 1 inch per this, against over BO2's loop (or half as much as against the other wrist).
const HELD_LOOSE: f32 = 3.0;

/// The run loop with each arm swinging back over its still half, or `None` when no bone jumps.
#[must_use]
pub fn smooth_loop_seam(clip: &AnimClip, skel: Option<&ViewSkeleton>) -> Option<AnimClip> {
    let n = usize::from(clip.numframes);
    if !clip.looping || n < 4 {
        return None;
    }
    // Each side's biggest jump picks the half that whole side swings back over. A held thing
    // follows its hand, so with the skeleton it picks nothing.
    let mut worst = 0.0f32;
    let mut sides: [Option<(f32, Half)>; 3] = [None; 3];
    for track in clip.tracks.iter().filter(|t| !own_motion(&t.name)) {
        let Some((turn, half)) = wrap_jump(track, n) else { continue };
        worst = worst.max(turn);
        if skel.is_some() && held(&track.name) {
            continue;
        }
        let side = &mut sides[side(&track.name) as usize];
        if !matches!(*side, Some((t, _)) if t >= turn) {
            *side = Some((turn, half));
        }
    }
    if worst <= CLIP_JUMP_DEG || worst <= STANDOUT_TIMES * biggest_inner_turn(clip, n) {
        return None;
    }
    let mut out = clip.clone();
    for track in out.tracks.iter_mut().filter(|t| !own_motion(&t.name)) {
        let half = match side(&track.name) {
            // The camera and the view's root are in no hand: one moves only if it jumps itself.
            Side::View => wrap_jump(track, n).map(|(_, h)| h),
            s => sides[s as usize].map(|(_, h)| h),
        };
        match (half, skel) {
            (Some(half), Some(_)) => ease_back(track, n, half),
            (Some(half), None) => replay(track, n, half),
            (None, _) => {}
        }
    }
    if let Some(skel) = skel {
        hold_in_hands(clip, &mut out, skel, n, |s| sides[s as usize].map(|(_, h)| h));
    }
    Some(out)
}

/// What a hand holds: the gun's bones on the view, and the gun's own root.
fn held(bone: &str) -> bool {
    bone.starts_with("tag_weapon") || bone == "tag_origin"
}

/// Each held thing set, frame by frame, where BO2 has it in the hand that holds it: the hand
/// is the wrist it moves least against over BO2's frames, and over that arm's eased half it
/// eases in step with the arm. Held things go in skeleton order, so the gun's root is set on
/// the gun bone already set.
fn hold_in_hands(raw: &AnimClip, out: &mut AnimClip, skel: &ViewSkeleton, n: usize, half: impl Fn(Side) -> Option<Half>) {
    let tracks = skel.tracks_for(raw);
    let wrists: Vec<(usize, Side)> = [("j_wrist_ri", Side::Right), ("j_wrist_le", Side::Left)]
        .into_iter()
        .filter_map(|(name, side)| Some((skel.find(name)?, side)))
        .collect();
    let bo2: Vec<Vec<Place>> = (0..=n).map(|f| skel.pose(raw, &tracks, f)).collect();
    let mut things: Vec<(usize, usize)> = raw
        .tracks
        .iter()
        .zip(&tracks)
        .enumerate()
        .filter(|(_, (t, _))| held(&t.name))
        .filter_map(|(i, (_, bone))| Some(((*bone)?, i)))
        .collect();
    things.sort_unstable();
    for (bone, track) in things {
        let thing = &skel.bones[bone];
        let Some(parent) = thing.parent else { continue };
        if thing.scale.abs() < 1e-6 {
            continue;
        }
        let mut hands: Vec<(f32, usize, Side, Vec<Place>)> = wrists
            .iter()
            .map(|&(w, side)| {
                let in_hand: Vec<Place> = bo2.iter().map(|p| mul_rt(inv_rt(p[w]), p[bone])).collect();
                (looseness(&in_hand), w, side, in_hand)
            })
            .collect();
        hands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let other = hands.get(1).map_or(f32::INFINITY, |h| h.0);
        let Some((loose, wrist, side, in_hand)) = hands.into_iter().next() else { continue };
        if loose > HELD_LOOSE && loose * 2.0 > other {
            continue;
        }
        let in_hand = match half(side) {
            Some(h) => ease(&in_hand, n, h, blend_place),
            None => in_hand,
        };
        let now: Vec<Vec<Place>> = (0..=n).map(|f| skel.pose(out, &tracks, f)).collect();
        let mut turns = Vec::with_capacity(n + 1);
        let mut moves = Vec::with_capacity(n + 1);
        for f in 0..=n {
            let (q, t) = mul_rt(inv_rt(now[f][parent]), mul_rt(now[f][wrist], in_hand[f]));
            turns.push(q);
            moves.push(sub(t, thing.bind.1).map(|c| c / thing.scale));
        }
        keep_signs(&mut turns);
        let keys = &mut out.tracks[track];
        keys.rotation = Rotation::FullKeyed(Keyed { values: turns, frames: FrameIndices::Dense });
        keys.translation = Translation::FullKeyed(Keyed { values: moves, frames: FrameIndices::Dense });
    }
}

/// How far a held thing strays in the hand over the loop: its biggest turn per 10 degrees or
/// move per inch from where it sits at the loop's start.
fn looseness(in_hand: &[Place]) -> f32 {
    let (q0, t0) = in_hand[0];
    in_hand.iter().map(|&(q, t)| (turn_deg(q, q0) / 10.0).max(dist(t, t0))).fold(0.0, f32::max)
}

/// Bones some guns carry whose wrap is part of their own motion (an LMG's belt bullets, a
/// launcher's strap): left as BO2 has them.
fn own_motion(bone: &str) -> bool {
    ["tag_bullet", "j_strap", "j_bullet", "j_belt"].iter().any(|p| bone.starts_with(p))
}

/// Which arm a bone moves with. The gun and its parts go with the right arm that holds it.
#[derive(Clone, Copy)]
enum Side {
    Right,
    Left,
    View,
}

fn side(bone: &str) -> Side {
    const VIEW: [&str; 6] = ["tag_camera", "origin_animate_jnt", "tag_gasmask", "tag_view", "tag_torso", "tag_cambone"];
    if bone.ends_with("_le") || bone.contains("_le_") {
        Side::Left
    } else if VIEW.contains(&bone) {
        Side::View
    } else {
        Side::Right
    }
}

/// A bone's wrap jump: its turn (degrees; 0 when it only moves) and the half it holds still
/// in, or `None` when its wrap is no bigger than its usual step.
fn wrap_jump(track: &Track, n: usize) -> Option<(f32, Half)> {
    let mut still = None;
    if let Rotation::HalfKeyed(keys) | Rotation::FullKeyed(keys) = &track.rotation {
        let frames: Vec<Quat> = (0..=n).map(|f| quat_at(keys, f)).collect();
        let jump = turn_deg(frames[n - 1], frames[0]);
        let half = still_half(&frames, turn_deg);
        if jump > BONE_JUMP_DEG && jump > USUAL_STEP_TIMES * usual(&frames, turn_deg) {
            return Some((jump, half));
        }
        // A bone's move swings back over the same half as its turn.
        still = Some(half);
    }
    if let Translation::SmallKeyed(keys) | Translation::FullKeyed(keys) = &track.translation {
        let frames: Vec<Vec3> = (0..=n).map(|f| vec_at(keys, f)).collect();
        let jump = dist(frames[n - 1], frames[0]);
        if jump > BONE_JUMP_UNITS && jump > USUAL_STEP_TIMES * usual(&frames, dist) {
            return Some((0.0, still.unwrap_or_else(|| still_half(&frames, dist))));
        }
    }
    None
}

/// The biggest turn (degrees) any bone makes in one frame inside the loop, the wrap left out.
fn biggest_inner_turn(clip: &AnimClip, n: usize) -> f32 {
    clip.tracks
        .iter()
        .filter(|t| !own_motion(&t.name))
        .filter_map(|t| match &t.rotation {
            Rotation::HalfKeyed(keys) | Rotation::FullKeyed(keys) => Some(keys),
            _ => None,
        })
        .flat_map(|keys| (0..n - 1).map(move |f| turn_deg(quat_at(keys, f), quat_at(keys, f + 1))))
        .fold(0.0, f32::max)
}

/// The bone keeping BO2's swing and easing back over its still half: its turn and its move.
fn ease_back(track: &mut Track, n: usize, still: Half) {
    if let Some(frames) = rot_frames(track, n) {
        let mut values = ease(&frames, n, still, slerp);
        keep_signs(&mut values);
        track.rotation = Rotation::FullKeyed(Keyed { values, frames: FrameIndices::Dense });
    }
    if let Some(frames) = vec_frames(track, n) {
        let values = ease(&frames, n, still, lerp);
        track.translation = Translation::FullKeyed(Keyed { values, frames: FrameIndices::Dense });
    }
}

/// BO2's frames 0 to `n` with the still half eased (see `eased`).
fn ease<T: Copy>(frames: &[T], n: usize, still: Half, blend: fn(T, T, f32) -> T) -> Vec<T> {
    (0..=n)
        .map(|f| {
            let (a, b, u) = eased(f, n, still);
            blend(frames[a], frames[b], u)
        })
        .collect()
}

/// What loop frame `f` shows of a bone easing back over its still half: two of BO2's frames
/// and how far from the first to the second (gently started and stopped). A bone still over
/// the second half eases from the middle frame, where its swing ends, back to frame 0. One
/// still over the first half eases from the frame before the wrap, where its swing ends,
/// across the wrap to the middle frame, where it starts. Its swing half is BO2's own frames,
/// and the loop's last frame is its first.
fn eased(f: usize, n: usize, still: Half) -> (usize, usize, f32) {
    let mid = n / 2;
    let gentle = |p: f32| p * p * (3.0 - 2.0 * p);
    match still {
        Half::Second if f > mid => (mid, 0, gentle((f - mid) as f32 / (n - mid) as f32)),
        Half::First if f < mid || f == n => {
            let steps = if f == n { 1 } else { f + 1 };
            (n - 1, mid, gentle(steps as f32 / (mid + 1) as f32))
        }
        _ => (f, f, 0.0),
    }
}

/// The bone playing `replay_frame`'s BO2 frames: its turn and its move.
fn replay(track: &mut Track, n: usize, still: Half) {
    let at: Vec<f32> = (0..=n).map(|f| replay_frame(f, n, still)).collect();
    if let Some(frames) = rot_frames(track, n) {
        let mut values: Vec<Quat> = at.iter().map(|&t| rot_at(&frames, t)).collect();
        keep_signs(&mut values);
        track.rotation = Rotation::FullKeyed(Keyed { values, frames: FrameIndices::Dense });
    }
    if let Some(frames) = vec_frames(track, n) {
        let values = at.iter().map(|&t| vec_lerp_at(&frames, t)).collect();
        track.translation = Translation::FullKeyed(Keyed { values, frames: FrameIndices::Dense });
    }
}

/// A keyed turn's BO2 frames 0 to `n`, or `None` when the bone does not turn.
fn rot_frames(track: &Track, n: usize) -> Option<Vec<Quat>> {
    let (Rotation::HalfKeyed(keys) | Rotation::FullKeyed(keys)) = &track.rotation else {
        return None;
    };
    Some((0..=n).map(|f| quat_at(keys, f)).collect())
}

fn vec_frames(track: &Track, n: usize) -> Option<Vec<Vec3>> {
    let (Translation::SmallKeyed(keys) | Translation::FullKeyed(keys)) = &track.translation else {
        return None;
    };
    Some((0..=n).map(|f| vec_at(keys, f)).collect())
}

/// A track's turn at a whole frame; none (no turn) when it has no turn.
fn turn_at(rotation: &Rotation, f: usize) -> Quat {
    match rotation {
        Rotation::Default => [0.0, 0.0, 0.0, 1.0],
        Rotation::HalfKeyed(keys) | Rotation::FullKeyed(keys) => quat_at(keys, f),
        Rotation::HalfConstant(q) | Rotation::FullConstant(q) => normalize(*q),
    }
}

fn move_at(translation: &Translation, f: usize) -> Vec3 {
    match translation {
        Translation::Default => [0.0; 3],
        Translation::SmallKeyed(keys) | Translation::FullKeyed(keys) => vec_at(keys, f),
        Translation::Constant(v) => *v,
    }
}

/// The frame a time falls in, and how far toward the next frame it is.
fn split(t: f32, n: usize) -> (usize, f32) {
    let i = (t.floor() as usize).min(n - 1);
    (i, t - i as f32)
}

fn rot_at(frames: &[Quat], t: f32) -> Quat {
    let (i, u) = split(t, frames.len() - 1);
    let (a, mut b) = (frames[i], frames[i + 1]);
    if dot(a, b) < 0.0 {
        b = b.map(|c| -c);
    }
    normalize([0, 1, 2, 3].map(|k| a[k] + u * (b[k] - a[k])))
}

fn vec_lerp_at(frames: &[Vec3], t: f32) -> Vec3 {
    let (i, u) = split(t, frames.len() - 1);
    lerp(frames[i], frames[i + 1], u)
}

fn lerp(a: Vec3, b: Vec3, u: f32) -> Vec3 {
    [0, 1, 2].map(|k| a[k] + u * (b[k] - a[k]))
}

/// The turn `u` of the way from `a` to `b`, the short way round at an even pace.
fn slerp(a: Quat, b: Quat, u: f32) -> Quat {
    let (mut b, mut d) = (b, dot(a, b));
    if d < 0.0 {
        b = b.map(|c| -c);
        d = -d;
    }
    if d > 0.9995 {
        return normalize([0, 1, 2, 3].map(|k| a[k] + u * (b[k] - a[k])));
    }
    let angle = d.min(1.0).acos();
    let (wa, wb) = (((1.0 - u) * angle).sin() / angle.sin(), (u * angle).sin() / angle.sin());
    normalize([0, 1, 2, 3].map(|k| wa * a[k] + wb * b[k]))
}

fn blend_place(a: Place, b: Place, u: f32) -> Place {
    (slerp(a.0, b.0, u), lerp(a.1, b.1, u))
}

/// Each turn on the same side as the one before, so the keys blend the short way.
fn keep_signs(values: &mut [Quat]) {
    for i in 1..values.len() {
        if dot(values[i - 1], values[i]) < 0.0 {
            values[i] = values[i].map(|c| -c);
        }
    }
}

/// Which of BO2's frames (a fraction between two) loop frame `f` shows, played back the other
/// way over the still half. A bone still over the second half plays its first half, then
/// plays it back to the start. One still over the first half plays its second half up to the
/// frame before the wrap, then back to its middle across the wrap. Either way the loop's last
/// frame is its first.
fn replay_frame(f: usize, n: usize, still: Half) -> f32 {
    let (f, n, mid) = (f as f32, n as f32, (n / 2) as f32);
    match still {
        Half::Second if f > mid => mid * (n - f) / (n - mid),
        Half::First if f < mid || f > n - 1.0 => {
            let f = if f > mid { 0.0 } else { f };
            mid + (n - 1.0 - mid) * (mid - f) / (mid + 1.0)
        }
        _ => f,
    }
}

/// The half of the loop a bone holds still in (moves least), where it eases back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Half {
    First,
    Second,
}

fn still_half<T: Copy>(frames: &[T], step: fn(T, T) -> f32) -> Half {
    let n = frames.len() - 1;
    let mid = n / 2;
    let moved = |a: usize, b: usize| frames[a..=b].windows(2).map(|w| step(w[0], w[1])).sum::<f32>();
    if moved(0, mid) < moved(mid, n - 1) { Half::First } else { Half::Second }
}

/// The middle size of one frame's step over the loop (wrap left out).
fn usual<T: Copy>(frames: &[T], step: fn(T, T) -> f32) -> f32 {
    let mut steps: Vec<f32> = frames.windows(2).map(|w| step(w[0], w[1])).collect();
    steps.pop();
    steps.sort_by(f32::total_cmp);
    steps.get(steps.len() / 2).copied().unwrap_or(0.0)
}

/// A keyed track's value at a whole frame, as the clip plays it (keys blended in a straight line).
fn key_span<T>(keys: &Keyed<T>, f: usize) -> (usize, usize, f32) {
    let last = keys.values.len().saturating_sub(1);
    match &keys.frames {
        FrameIndices::Dense => (f.min(last), (f + 1).min(last), 0.0),
        FrameIndices::Sparse(at) => {
            let at = |i: usize| usize::from(at.get(i).copied().unwrap_or(0));
            if f <= at(0) {
                return (0, 0, 0.0);
            }
            if f >= at(last) {
                return (last, last, 0.0);
            }
            let i = (0..last).find(|&i| f <= at(i + 1)).unwrap_or(last);
            let (a, b) = (at(i), at(i + 1).max(at(i) + 1));
            (i, (i + 1).min(last), (f - a) as f32 / (b - a) as f32)
        }
    }
}

fn quat_at(keys: &Keyed<Quat>, f: usize) -> Quat {
    let (a, b, t) = key_span(keys, f);
    let (a, b) = (keys.values[a], keys.values[b]);
    normalize([0, 1, 2, 3].map(|i| a[i] + t * (b[i] - a[i])))
}

fn vec_at(keys: &Keyed<Vec3>, f: usize) -> Vec3 {
    let (a, b, t) = key_span(keys, f);
    let (a, b) = (keys.values[a], keys.values[b]);
    [0, 1, 2].map(|i| a[i] + t * (b[i] - a[i]))
}

fn dot(a: Quat, b: Quat) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn turn_deg(a: Quat, b: Quat) -> f32 {
    2.0 * dot(a, b).abs().min(1.0).acos().to_degrees()
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    let d = sub(a, b);
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalize(q: Quat) -> Quat {
    let len = dot(q, q).sqrt();
    if len > 1e-6 { q.map(|c| c / len) } else { [0.0, 0.0, 0.0, 1.0] }
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// One turn after another (x, y, z, w), as the skeleton puts a child under its parent.
fn mul_q(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn turn_vec(q: Quat, v: Vec3) -> Vec3 {
    let p = mul_q(mul_q(q, [v[0], v[1], v[2], 0.0]), [-q[0], -q[1], -q[2], q[3]]);
    [p[0], p[1], p[2]]
}

/// A child bone's place under its parent's.
fn mul_rt(parent: Place, child: Place) -> Place {
    (mul_q(parent.0, child.0), add(parent.1, turn_vec(parent.0, child.1)))
}

fn inv_rt((q, t): Place) -> Place {
    let c = [-q[0], -q[1], -q[2], q[3]];
    (c, turn_vec(c, t).map(|x| -x))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about_z(deg: f32) -> Quat {
        let h = deg.to_radians() / 2.0;
        [0.0, 0.0, h.sin(), h.cos()]
    }

    fn deg(q: Quat) -> f32 {
        let q = if q[3] < 0.0 { q.map(|c| -c) } else { q };
        2.0 * q[2].atan2(q[3]).to_degrees()
    }

    /// The ballistic knife's right arm as BO2 keys it: forward over frames 0-15, held to 29,
    /// back to the start at 30, and the knife moving with it. The camera bobs smoothly the
    /// whole loop; the left shoulder is the mirror of the right arm: still to 15, forward to
    /// 29, back at 30.
    fn knife_run() -> AnimClip {
        let mut fr: Vec<u16> = (0..=15).collect();
        fr.extend([29, 30]);
        let mut arm: Vec<Quat> = (0..=15).map(|f| about_z(f as f32 * 6.0)).collect();
        arm.extend([about_z(90.0), about_z(0.0)]);
        let mut gun: Vec<Vec3> = (0..=15).map(|f| [f as f32, 0.0, 0.0]).collect();
        gun.extend([[15.0, 0.0, 0.0], [0.0, 0.0, 0.0]]);
        let left: Vec<Quat> = (0..=30)
            .map(|f| about_z(20.0 * (f as f32 / 30.0 * std::f32::consts::TAU).sin()))
            .collect();
        let shoulder: Vec<Quat> = (0..=30u16)
            .map(|f| about_z(if f == 30 { 0.0 } else { f.saturating_sub(15) as f32 * 6.0 }))
            .collect();
        AnimClip {
            name: "knife_run".to_owned(),
            framerate: 30.0,
            numframes: 30,
            looping: true,
            tracks: vec![
                Track {
                    name: "j_elbow_ri".to_owned(),
                    rotation: Rotation::HalfKeyed(Keyed {
                        values: arm,
                        frames: FrameIndices::Sparse(fr.clone()),
                    }),
                    translation: Translation::Default,
                },
                Track {
                    name: "tag_weapon".to_owned(),
                    rotation: Rotation::Default,
                    translation: Translation::FullKeyed(Keyed {
                        values: gun,
                        frames: FrameIndices::Sparse(fr),
                    }),
                },
                Track {
                    name: "tag_camera".to_owned(),
                    rotation: Rotation::HalfKeyed(Keyed {
                        values: left,
                        frames: FrameIndices::Dense,
                    }),
                    translation: Translation::Default,
                },
                Track {
                    name: "j_shoulder_le".to_owned(),
                    rotation: Rotation::HalfKeyed(Keyed {
                        values: shoulder,
                        frames: FrameIndices::Dense,
                    }),
                    translation: Translation::Default,
                },
            ],
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        }
    }

    fn turns(clip: &AnimClip, track: usize) -> Vec<Quat> {
        let (Rotation::HalfKeyed(k) | Rotation::FullKeyed(k)) = &clip.tracks[track].rotation else {
            panic!("keyed");
        };
        (0..=30).map(|f| quat_at(k, f)).collect()
    }

    /// The biggest turn in one frame over the whole loop, the wrap included.
    fn biggest_step(q: &[Quat]) -> f32 {
        let wrap = turn_deg(q[q.len() - 1], q[0]);
        q.windows(2).map(|w| turn_deg(w[0], w[1])).fold(wrap, f32::max)
    }

    #[test]
    fn the_knife_arm_swings_back_instead_of_jumping() {
        let before = knife_run();
        assert!(biggest_step(&turns(&before, 0)) > 89.0, "BO2's own data jumps 90 at the wrap");
        let after = smooth_loop_seam(&before, None).expect("the knife loop jumps");
        let arm = turns(&after, 0);
        assert!(biggest_step(&arm) < 13.0, "biggest step {}", biggest_step(&arm));
        // The forward swing is BO2's own.
        let own = turns(&before, 0);
        for f in 0..=15 {
            assert!(turn_deg(arm[f], own[f]) < 0.01, "frame {f}");
        }
        // The gun comes back with the arm.
        let Translation::FullKeyed(g) = &after.tracks[1].translation else {
            panic!("smoothed")
        };
        let steps: Vec<f32> = (0..30).map(|f| dist(vec_at(g, f), vec_at(g, f + 1))).collect();
        assert!(steps.iter().all(|&s| s < 2.5), "{steps:?}");
        assert!(dist(vec_at(g, 30), [0.0; 3]) < 1e-4);
    }

    /// His 10-20 picture: the knife sat off the hand mid-stride. Without the skeleton, the hand
    /// and the knife show the same moment of BO2's swing in every frame (the arm turns 6 a
    /// frame while the knife moves 1).
    #[test]
    fn the_knife_stays_in_the_hand_every_frame() {
        let after = smooth_loop_seam(&knife_run(), None).expect("the knife loop jumps");
        let arm = turns(&after, 0);
        let Translation::FullKeyed(g) = &after.tracks[1].translation else {
            panic!("replayed")
        };
        for f in 0..=30 {
            let knife = vec_at(g, f)[0];
            assert!((deg(arm[f]) / 6.0 - knife).abs() < 0.01, "frame {f}: hand at {} knife at {knife}", deg(arm[f]) / 6.0);
        }
    }

    #[test]
    fn the_left_arm_swings_back_while_it_would_hold() {
        let before = knife_run();
        let after = smooth_loop_seam(&before, None).expect("the knife loop jumps");
        let arm = turns(&after, 3);
        assert!(biggest_step(&arm) < 13.0, "biggest step {}", biggest_step(&arm));
        // Its forward swing (the second half) is BO2's own.
        let own = turns(&before, 3);
        for f in 15..30 {
            assert!(turn_deg(arm[f], own[f]) < 0.01, "frame {f}");
        }
    }

    #[test]
    fn a_bone_that_already_loops_is_untouched() {
        let after = smooth_loop_seam(&knife_run(), None).expect("the knife loop jumps");
        assert!(matches!(after.tracks[2].rotation, Rotation::HalfKeyed(_)));
    }

    #[test]
    fn a_rifle_loop_that_ends_where_it_starts_is_left_as_bo2_has_it() {
        let mut rifle = knife_run();
        rifle.tracks.truncate(0);
        rifle.tracks.push(knife_run().tracks.remove(2));
        assert!(smooth_loop_seam(&rifle, None).is_none());
        let mut once = knife_run();
        once.looping = false;
        assert!(smooth_loop_seam(&once, None).is_none());
        assert!(smooth_loop_seam(&rifle, Some(&view())).is_none());
    }

    /// A small view: two arms on the view's root, each a shoulder and a wrist; the gun bone
    /// on the view; the gun (its root `j_gun` on the gun bone, its own root `tag_origin`).
    fn view() -> ViewSkeleton {
        let bone = |name: &str, parent: Option<usize>, t: Vec3| SkelBone {
            name: name.to_owned(),
            parent,
            bind: ([0.0, 0.0, 0.0, 1.0], t),
            scale: 1.0,
        };
        ViewSkeleton {
            bones: vec![
                bone("tag_view", None, [0.0; 3]),
                bone("j_shoulder_ri", Some(0), [0.0, -5.0, 0.0]),
                bone("j_wrist_ri", Some(1), [10.0, 0.0, 0.0]),
                bone("tag_weapon", Some(0), [0.0; 3]),
                bone("j_shoulder_le", Some(0), [0.0, 5.0, 0.0]),
                bone("j_wrist_le", Some(4), [10.0, 0.0, 0.0]),
                bone("j_gun", Some(3), [0.0; 3]),
                bone("tag_origin", Some(6), [0.0, 4.63, 0.0]),
            ],
            duplicates: Vec::new(),
        }
    }

    /// One arm's swing as BO2 keys it, from 0 to 1 over its swing half: back to about -42
    /// degrees, then forward to 60.
    fn swing(k: f32) -> f32 {
        60.0 * k - 70.0 * (std::f32::consts::PI * k).sin()
    }

    /// The ballistic knife's run as BO2 keys it on the small view: the right arm swings over
    /// frames 0-15 and holds to 29, the left holds to 15 and swings over 15-29, both jump back
    /// at 30. The gun bone follows the right wrist; the knife (the gun's own root) sits in the
    /// left fist.
    fn knife_on_view() -> AnimClip {
        let right: Vec<f32> = (0..=30).map(|f| if f == 30 { 0.0 } else { swing((f.min(15) as f32) / 15.0) }).collect();
        let left: Vec<f32> =
            (0..=30).map(|f| if f == 30 || f <= 15 { 0.0 } else { swing((f - 15) as f32 / 14.0) }).collect();
        let skel = view();
        let arm = |deg: f32, shoulder: Vec3| mul_rt((about_z(deg), shoulder), ([0.0, 0.0, 0.0, 1.0], [10.0, 0.0, 0.0]));
        let mut gun_turns = Vec::new();
        let mut gun_moves = Vec::new();
        let mut knife_turns = Vec::new();
        let mut knife_moves = Vec::new();
        for f in 0..=30 {
            let gun = mul_rt(arm(right[f], [0.0, -5.0, 0.0]), ([0.0, 0.0, 0.0, 1.0], [3.0, 0.0, 0.0]));
            let fist = mul_rt(arm(left[f], [0.0, 5.0, 0.0]), (about_z(30.0), [2.0, 0.0, 0.0]));
            let knife = mul_rt(inv_rt(gun), fist);
            gun_turns.push(gun.0);
            gun_moves.push(gun.1);
            knife_turns.push(knife.0);
            knife_moves.push(sub(knife.1, skel.bones[7].bind.1));
        }
        let keyed = |name: &str, turns: Vec<Quat>, moves: Option<Vec<Vec3>>| Track {
            name: name.to_owned(),
            rotation: Rotation::FullKeyed(Keyed { values: turns, frames: FrameIndices::Dense }),
            translation: moves.map_or(Translation::Default, |values| {
                Translation::FullKeyed(Keyed { values, frames: FrameIndices::Dense })
            }),
        };
        AnimClip {
            name: "knife_on_view".to_owned(),
            framerate: 30.0,
            numframes: 30,
            looping: true,
            tracks: vec![
                keyed("j_shoulder_ri", right.iter().map(|&d| about_z(d)).collect(), None),
                keyed("j_shoulder_le", left.iter().map(|&d| about_z(d)).collect(), None),
                keyed("tag_weapon", gun_turns, Some(gun_moves)),
                keyed("tag_origin", knife_turns, Some(knife_moves)),
            ],
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        }
    }

    /// Each arm's swing angle over the loop, as posed.
    fn arm_angles(clip: &AnimClip, shoulder: usize) -> Vec<f32> {
        let skel = view();
        let tracks = skel.tracks_for(clip);
        (0..=30).map(|f| deg(skel.pose(clip, &tracks, f)[shoulder].0)).collect()
    }

    /// The frames (of 0-29) where an arm is swung back past -30 degrees, as runs of frames
    /// in a row (the wrap joins the last run to the first).
    fn back_swings(angles: &[f32]) -> Vec<Vec<usize>> {
        let back: Vec<bool> = angles[..30].iter().map(|&a| a < -30.0).collect();
        let mut runs: Vec<Vec<usize>> = Vec::new();
        for f in 0..30 {
            if back[f] {
                if f > 0 && back[f - 1] {
                    runs.last_mut().expect("run").push(f);
                } else {
                    runs.push(vec![f]);
                }
            }
        }
        if runs.len() > 1 && back[0] && back[29] {
            let first = runs.remove(0);
            runs.last_mut().expect("run").extend(first);
        }
        runs
    }

    /// His 10-20 run: both arms rocked back together, twice a stride. Each arm now swings back
    /// once a stride, the two half a stride apart, and never jumps.
    #[test]
    fn the_arms_swing_back_in_turn_once_a_stride() {
        let before = knife_on_view();
        let after = smooth_loop_seam(&before, Some(&view())).expect("the knife loop jumps");
        let (right, left) = (arm_angles(&after, 1), arm_angles(&after, 4));
        let (rs, ls) = (back_swings(&right), back_swings(&left));
        assert_eq!(rs.len(), 1, "right arm back-swings {rs:?} {right:?}");
        assert_eq!(ls.len(), 1, "left arm back-swings {ls:?} {left:?}");
        let middle = |run: &[usize]| run[run.len() / 2] as i32;
        let apart = (middle(&rs[0]) - middle(&ls[0])).rem_euclid(30);
        assert!((10..=20).contains(&apart), "back-swings {apart} frames apart: {rs:?} {ls:?}");
        // No step bigger than BO2's own swing makes (BO2's wrap jumped 60), and the loop's last
        // frame is its first.
        let (r0, l0) = (arm_angles(&before, 1), arm_angles(&before, 4));
        for (arm, own) in [(&right, &r0), (&left, &l0)] {
            let own_biggest = own[..30].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            let biggest = arm.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            assert!(biggest <= own_biggest + 0.01, "biggest step {biggest} past BO2's {own_biggest}: {arm:?}");
            assert!((arm[30] - arm[0]).abs() < 0.01);
        }
        // Each arm's swing half is BO2's own.
        for f in 0..=15 {
            assert!((right[f] - r0[f]).abs() < 0.01, "right frame {f}");
        }
        for f in 15..30 {
            assert!((left[f] - l0[f]).abs() < 0.01, "left frame {f}");
        }
        // Played backward instead, each arm swings back twice a stride.
        let replayed = smooth_loop_seam(&before, None).expect("the knife loop jumps");
        assert_eq!(back_swings(&arm_angles(&replayed, 1)).len(), 2);
    }

    /// The gun bone stays in the right hand and the knife in the left fist, exactly where BO2
    /// has them, in every frame of the eased loop.
    #[test]
    fn the_gun_and_the_knife_stay_in_their_hands_every_frame() {
        let skel = view();
        let after = smooth_loop_seam(&knife_on_view(), Some(&skel)).expect("the knife loop jumps");
        let tracks = skel.tracks_for(&after);
        let mut knife_at = Vec::new();
        for f in 0..=30 {
            let pose = skel.pose(&after, &tracks, f);
            let gun = mul_rt(inv_rt(pose[2]), pose[3]);
            assert!(turn_deg(gun.0, [0.0, 0.0, 0.0, 1.0]) < 0.2, "frame {f}: gun turned {}", deg(gun.0));
            assert!(dist(gun.1, [3.0, 0.0, 0.0]) < 0.01, "frame {f}: gun at {:?}", gun.1);
            let knife = mul_rt(inv_rt(pose[5]), pose[7]);
            assert!(turn_deg(knife.0, about_z(30.0)) < 0.2, "frame {f}: knife turned {}", deg(knife.0));
            assert!(dist(knife.1, [2.0, 0.0, 0.0]) < 0.01, "frame {f}: knife at {:?}", knife.1);
            knife_at.push(pose[7].1);
        }
        // The knife comes round with the fist: no jump at the wrap (BO2's own swing moves the
        // fist up to about 4 a frame; its wrap jumped 12).
        let steps: Vec<f32> = knife_at.windows(2).map(|w| dist(w[0], w[1])).collect();
        assert!(steps.iter().all(|&s| s < 5.0), "{steps:?}");
        assert!(dist(knife_at[30], knife_at[0]) < 1e-3);
    }

    /// A pistol's run as BO2 keys its wrist: a 50 degree wrap, but a 40 degree flick mid-loop.
    fn pistol_run(flick: bool) -> AnimClip {
        let wrist: Vec<Quat> = (0..=30u16)
            .map(|f| match f {
                30 => about_z(0.0),
                f if flick && f >= 10 => about_z(50.0),
                f if flick => about_z(f32::from(f)),
                f => about_z(f32::from(f) * 50.0 / 29.0),
            })
            .collect();
        AnimClip {
            name: "pistol_run".to_owned(),
            framerate: 30.0,
            numframes: 30,
            looping: true,
            tracks: vec![Track {
                name: "j_wrist_ri".to_owned(),
                rotation: Rotation::HalfKeyed(Keyed { values: wrist, frames: FrameIndices::Dense }),
                translation: Translation::Default,
            }],
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        }
    }

    #[test]
    fn a_pistol_wrap_no_bigger_than_its_own_flicks_is_left_as_bo2_has_it() {
        let clip = pistol_run(true);
        assert!(smooth_loop_seam(&clip, None).is_none());
        assert!(smooth_loop_seam(&clip, Some(&view())).is_none());
        // The same 50 degree wrap after a slow swing does stand out, and is smoothed.
        assert!(smooth_loop_seam(&pistol_run(false), None).is_some());
    }
}
