//! Names floated over other players' heads: who qualifies, how long each
//! stays up, and the rows handed to `ui` to draw.

use std::collections::HashMap;

use bevy::prelude::*;
use hud_t6::flashbang::{ShockViewType, cg_flashbanged};
use hud_t6::font::GLYPH_EM_UNIT;
use hud_t6::overhead_names::*;
use net::{ClientEntityRecord, ClientEntityPoseState, FrameClock, HostViewClient, ShownWorldFrame};

use crate::gaps::GapReason;
use crate::gaps::HudGapBook;
use crate::gaps::HudShortage;
use crate::draw_batch::LayerRequests;
use crate::draw_batch::LayerJob;
use crate::surface::FlatOverlayCanvas;
use frame::LifeStatePosted;

/// Where the render side found a player's head this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NameplateSkullPose {
    TrueSceneSpace([f32; 3]),
    MissingBodyOrSkull,
}

/// The posed heads of the frame, by entity number.
#[derive(Resource, Debug, Default)]
pub struct NameplateBodyPose {
    by_entity: HashMap<u16, NameplateSkullPose>,
}

impl NameplateBodyPose {
    pub fn replace(&mut self, poses: impl IntoIterator<Item = (u16, NameplateSkullPose)>) {
        self.by_entity.clear();
        self.by_entity.extend(poses);
    }

    pub(crate) fn get(&self, entnum: u16) -> Option<NameplateSkullPose> {
        self.by_entity.get(&entnum).copied()
    }
}

/// Ordering point: the posed heads are final for the frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameplateBodyPoseShared;

#[derive(Component)]
pub(crate) struct OverheadNamesRaster;

/// One name on screen: since when, until when, and which side it was shown
/// for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Showing {
    since: i32,
    latest: i32,
    friendly: bool,
}

impl Showing {
    fn starting(now: i32, friendly: bool) -> Self {
        Self {
            since: now,
            latest: now,
            friendly,
        }
    }
}

/// The names currently up, by entity number, and the clock they were last
/// stepped at.
#[derive(Default)]
pub(crate) struct NameBook {
    stepped_at: Option<i32>,
    up: HashMap<u16, Showing>,
}

impl NameBook {
    /// Notes the frame time; a clock that went backwards drops everything.
    fn tick(&mut self, now: i32) {
        if self.stepped_at.is_some_and(|before| now < before) {
            self.up.clear();
        }
        self.stepped_at = Some(now);
    }

    /// Keeps only the entities in `present`.
    fn keep_only(&mut self, present: impl Fn(u16) -> bool) {
        self.up.retain(|ent, _| present(*ent));
    }

    /// Moves one entity's name on by a frame. `earned` says whether it
    /// qualifies this frame. Gives the alpha to draw it with, or `None` when
    /// it is not up (or has faded away, which also forgets it).
    fn step(&mut self, ent: u16, friendly: bool, earned: bool, now: i32) -> Option<f32> {
        if earned {
            let entry = self.up.entry(ent).or_insert_with(|| Showing::starting(now, friendly));
            if entry.friendly != friendly {
                *entry = Showing::starting(now, friendly);
            }
            entry.latest = now;
        }
        let shown = *self.up.get(&ent)?;
        if shown.friendly != friendly {
            self.up.remove(&ent);
            return None;
        }
        let fade_ms = if friendly {
            FRIENDLY_FADE_OUT_MS
        } else {
            ENEMY_FADE_IN_MS
        };
        if now.wrapping_sub(shown.latest) >= fade_ms {
            self.up.remove(&ent);
            return None;
        }
        let fade_in_ms = if friendly { 0 } else { ENEMY_FADE_IN_MS };
        let alpha = overhead_fade_alpha(now, shown.since, shown.latest, fade_in_ms, fade_ms);
        (alpha > 0.0).then_some(alpha)
    }
}

pub(crate) fn register(hud_app: &mut App) {
    hud_app.init_resource::<NameplateBodyPose>().add_systems(
        Update,
        update_overhead_names
            .after(NameplateBodyPoseShared)
            .after(crate::surface::refresh_hud_surface)
            .before(crate::plugin::flush_overhead_names_tess)
            .before(crate::gaps::log_hud_shortages)
            .in_set(LifeStatePosted),
    );
}

const TEAM_SPECTATOR_GREY: [f32; 4] = [0.25, 0.25, 0.25, 1.0];
const TEAM_ALLY_GREEN: [f32; 4] = [0.4, 0.7, 0.4, 1.0];
const TEAM_OPPONENT_RED: [f32; 4] = [1.0, 0.315, 0.35, 1.0];

/// The colour a name is drawn in, chosen relative to the viewer: own side,
/// the other side, or a spectator. The kill feed's tan and blue-grey are the
/// faction colours, a different pair.
fn relative_team_color(local_team: i32, target_team: i32) -> [f32; 4] {
    if target_team == entity_t6::team::SPECTATOR {
        TEAM_SPECTATOR_GREY
    } else if local_team != 0 && local_team == target_team {
        TEAM_ALLY_GREEN
    } else {
        TEAM_OPPONENT_RED
    }
}

/// A name with its colour escapes and control characters taken out, so a
/// player cannot dress up as a friend. Removing an escape can join an earlier
/// insert_column to a later digit (`^^12`); that pair goes too.
fn plain_name(name: &str) -> String {
    let mut rest = name.chars().peekable();
    let mut plain = String::new();
    while let Some(c) = rest.next() {
        if c == '^' && rest.peek().is_some_and(char::is_ascii_digit) {
            rest.next();
        } else if c.is_control() {
            // dropped
        } else if c.is_ascii_digit() && plain.ends_with('^') {
            plain.pop();
        } else {
            plain.push(c);
        }
    }
    plain
}

/// The other living players a HUD pass may mark (these names, the target
/// finder's diamonds): present in the next snapshot, a player entity, not
/// hidden or dead, not the local client, and alive in the meta.
pub(crate) fn markable_player<'s>(
    snapshot: &'s sim::Snapshot,
    me: sim::ClientId,
    ident: &ClientEntityRecord,
    live: &ClientEntityPoseState,
) -> Option<(sim::ClientId, &'s sim::ClientSnapshotMeta)> {
    const HIDDEN_FLAG: u32 = 0x20;
    let next = &live.upcoming_condition;
    let excluded = HIDDEN_FLAG | playerstate_t6::game::entity_flag_bits::DEAD;
    let a_player = live.present_in_newer_frame() && next.e_type == entity_t6::et::PLAYER;
    if !a_player || next.lerp.e_flags as u32 & excluded != 0 {
        return None;
    }
    let other = ident.client().filter(|&client| client != me)?;
    let meta = snapshot.meta.for_client(other)?;
    (meta.lifecycle == sim::PlayerSessionStage::Alive).then_some((other, meta))
}

/// A world trace from `from` to `to` with the names' mask: how far it got
/// (as a fraction) and whether it began inside something solid.
pub(crate) fn tag_trace(world: &sim::SimWorld, from: Vec3, to: Vec3) -> (f32, bool) {
    let result = world.trace_world(
        from.to_array(),
        to.to_array(),
        [0.0; 3],
        [0.0; 3],
        NAMEPLATE_SIGHT_FILTER,
    );
    (result.fraction, result.starts_inside)
}

/// Nothing solid between the eye and the head.
pub(crate) fn clear_line_to(world: &sim::SimWorld, eye: Vec3, head: Vec3) -> bool {
    match tag_trace(world, eye, head) {
        (reached, false) => reached >= 1.0,
        (_, true) => false,
    }
}

/// How far along the view ray it enters the player's bounds (a box around the
/// origin and head, so crouching and prone are covered), if it does.
pub(crate) fn crosshair_hit_distance(
    eye: Vec3,
    forward: Vec3,
    feet: Vec3,
    head: Vec3,
) -> Option<f32> {
    let lo = feet.min(head) - Vec3::new(15.0, 15.0, 0.0);
    let hi = feet.max(head) + Vec3::new(15.0, 15.0, 8.0);
    let mut near = 0.0f32;
    let mut far = NAME_SCAN_RANGE;
    for axis in 0_usize..3 {
        if forward[axis].abs() < 1e-6 {
            // Parallel to this slab: inside it the whole way or never.
            if !(lo[axis]..=hi[axis]).contains(&eye[axis]) {
                return None;
            }
            continue;
        }
        let t_lo = (lo[axis] - eye[axis]) / forward[axis];
        let t_hi = (hi[axis] - eye[axis]) / forward[axis];
        near = near.max(t_lo.min(t_hi));
        far = far.min(t_lo.max(t_hi));
    }
    (near <= far).then_some(near)
}

/// A player who could get a name this frame.
struct Nominee {
    ent: u16,
    team: i32,
    shown_name: String,
    anchor: Vec3,
    pixel: Vec2,
    in_line: bool,
    crosshair: Option<f32>,
}

/// What the viewer sees through this frame.
struct Viewer<'a> {
    eye: Vec3,
    forward: Vec3,
    camera: &'a Camera,
    transform: &'a GlobalTransform,
    window: (f32, f32),
    world: &'a sim::SimWorld,
}

impl Viewer<'_> {
    /// Where a world point lands in the window, if it lands in it at all.
    fn on_window(&self, point: Vec3) -> Option<Vec2> {
        let pixel = self.camera.world_to_viewport(self.transform, point).ok()?;
        let inside = pixel.is_finite()
            && (0.0..=self.window.0).contains(&pixel.x)
            && (0.0..=self.window.1).contains(&pixel.y);
        inside.then_some(pixel)
    }
}

/// Head position for a player: the posed one, or a fallback above the origin.
/// `None` when the render side did not report the player at all.
fn head_position(posed: Option<NameplateSkullPose>, origin: Vec3) -> Option<Vec3> {
    match posed? {
        NameplateSkullPose::TrueSceneSpace(at) => Some(Vec3::from_array(at)),
        NameplateSkullPose::MissingBodyOrSkull => {
            Some(origin + Vec3::Z * (NAME_FALLBACK_HEIGHT - NAME_LIFT_ABOVE_HEAD))
        }
    }
}

/// Everyone close enough, on screen and named, with the facts the choosing
/// needs.
fn nominate(
    viewer: &Viewer,
    frame_snap: &sim::Snapshot,
    me: sim::ClientId,
    heads: &NameplateBodyPose,
    others: &Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    gaps: &mut HudGapBook,
) -> Vec<Nominee> {
    let mut found = Vec::new();
    for (ident, live) in others {
        let Some(meta) = markable_player(frame_snap, me, ident, live).map(|found| found.1) else {
            continue;
        };
        let name = entity_t6::client_info::c_str(&meta.name);
        if name.is_empty() {
            continue;
        }
        let ent = ident.number();
        let at_feet = Vec3::from_array(live.origin);
        let Some(head) = head_position(heads.get(ent), at_feet) else {
            gaps.log_gap(GapReason::TagHeadOff { entnum: ent });
            continue;
        };
        if viewer.eye.distance_squared(head) > NAME_RANGE_MAX.powi(2) {
            continue;
        }
        let anchor = head + Vec3::Z * NAME_LIFT_ABOVE_HEAD;
        let Some(pixel) = viewer.on_window(anchor) else {
            continue;
        };
        let in_line = clear_line_to(viewer.world, viewer.eye, head);
        found.push(Nominee {
            ent,
            team: meta.client_state_team,
            shown_name: plain_name(name),
            anchor,
            pixel,
            in_line,
            crosshair: crosshair_hit_distance(viewer.eye, viewer.forward, at_feet, head)
                .filter(|_| in_line),
        });
    }
    found
}

/// The nominee nearest along the crosshair, if any is under it.
fn under_crosshair(nominees: &[Nominee]) -> Option<u16> {
    nominees
        .iter()
        .filter_map(|n| n.crosshair.map(|distance| (n.ent, distance)))
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(ent, _)| ent)
}

/// One name's row for `ui`: sized by distance, faded, in the relative colour.
fn name_row(nominee: Nominee, alpha: f32, viewer_side: i32, viewer_eye: Vec3) -> ui::OverheadNameRow {
    let scale = overhead_distance_scale(
        viewer_eye.to_array(),
        nominee.anchor.to_array(),
        NAME_RANGE_NEAR,
        NAME_RANGE_FAR,
        NAME_FAR_SHRINK,
    );
    let [r, g, b, _] = relative_team_color(viewer_side, nominee.team);
    // The size is in text ems in window pixels (720p config: 0.65), and the
    // name's baseline sits on the head point.
    let px = NAME_TEXT_SIZE * GLYPH_EM_UNIT * scale;
    ui::OverheadNameRow {
        ent: nominee.ent,
        text: nominee.shown_name,
        x: nominee.pixel.x,
        y: nominee.pixel.y.round() - px * 0.5,
        px,
        color: [r, g, b, alpha],
    }
}

#[allow(clippy::too_many_arguments)]
fn update_overhead_names(
    shown_frame: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    heads: Res<NameplateBodyPose>,
    others: Query<(&ClientEntityRecord, &ClientEntityPoseState)>,
    clock: Res<FrameClock>,
    surface: Res<FlatOverlayCanvas>,
    view_cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    prediction: Option<Res<net::ClientGuessState>>,
    view: Res<frame::EyeSubject>,
    mut pass: ResMut<LayerRequests>,
    mut book: Local<NameBook>,
    mut gaps: ResMut<HudGapBook>,
    mut out: Option<ResMut<ui::OverheadNameRows>>,
) {
    pass.names = LayerJob::Conceal;
    if let Some(out) = out.as_deref_mut() {
        out.0.clear();
    }
    gaps.settle(HudShortage::NameTags);
    let now = clock.time();
    book.tick(now);

    let me = local.0;
    let Some(snapshot) = shown_frame.snapshot() else {
        book.up.clear();
        return;
    };
    if !surface.has_size() || view.replaying_final_death() {
        book.up.clear();
        return;
    }
    let Some(player) = shown_frame.living_participant(me) else {
        book.up.clear();
        return;
    };
    let Some(my_meta) = snapshot.meta.for_client(me) else {
        return;
    };
    let shock_kind = shown_frame
        .shellshock(me)
        .map_or(ShockViewType::Blurred, |shock| shock.screen_blend.kind);
    if cg_flashbanged(now, player.stun_start, player.stun_length, shock_kind) {
        book.up.clear();
        return;
    }
    let Some((camera, transform)) = view_cameras.iter().find(|(camera, _)| camera.is_active)
    else {
        return;
    };
    let Some(world) = prediction
        .as_ref()
        .filter(|state| state.0.is_forecast_live() && state.0.world().has_static_collision())
        .map(|state| state.0.world())
    else {
        gaps.log_gap(GapReason::TagVisibilityOff);
        return;
    };

    let viewer = Viewer {
        eye: transform.translation(),
        forward: *transform.forward(),
        camera,
        transform,
        window: (surface.width(), surface.height()),
        world,
    };
    let nominees = nominate(&viewer, snapshot, me, &heads, &others, &mut gaps);
    let aimed_at = under_crosshair(&nominees);
    book.keep_only(|ent| nominees.iter().any(|n| n.ent == ent));

    let my_team = my_meta.client_state_team;
    for nominee in nominees {
        let friendly = my_team != 0 && my_team == nominee.team;
        let earned = nominee.in_line && (friendly || aimed_at == Some(nominee.ent));
        let Some(alpha) = book.step(nominee.ent, friendly, earned, now) else {
            continue;
        };
        if let Some(out) = out.as_deref_mut() {
            out.0.push(name_row(nominee, alpha, my_team, viewer.eye));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EYE: Vec3 = Vec3::new(0.0, 0.0, 60.0);

    #[test]
    fn team_colours_depend_on_the_viewers_side() {
        let spectator = entity_t6::team::SPECTATOR;
        assert_eq!(relative_team_color(1, spectator), TEAM_SPECTATOR_GREY);
        assert_eq!(relative_team_color(1, 1), TEAM_ALLY_GREEN);
        assert_eq!(relative_team_color(1, 2), TEAM_OPPONENT_RED);
        // A viewer on no side has no allies.
        assert_eq!(relative_team_color(0, 0), TEAM_OPPONENT_RED);
    }

    #[test]
    fn plain_names_lose_escapes_and_controls() {
        assert_eq!(plain_name("^1Red^7Name"), "RedName");
        assert_eq!(plain_name("a\u{7}b\nc"), "abc");
        assert_eq!(plain_name("no escapes"), "no escapes");
        assert_eq!(plain_name("trailing^"), "trailing^");
    }

    #[test]
    fn a_caret_joined_to_a_digit_by_removal_goes_too() {
        assert_eq!(plain_name("^^12x"), "x");
        assert_eq!(plain_name("a^\u{1}5"), "a");
    }

    #[test]
    fn head_falls_back_above_the_origin() {
        let origin = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(head_position(None, origin), None);
        let exact = NameplateSkullPose::TrueSceneSpace([4.0, 5.0, 6.0]);
        assert_eq!(head_position(Some(exact), origin), Some(Vec3::new(4.0, 5.0, 6.0)));
        let fallback = head_position(Some(NameplateSkullPose::MissingBodyOrSkull), origin).unwrap();
        assert_eq!(fallback.truncate(), origin.truncate());
        assert_eq!(fallback.z, 3.0 + NAME_FALLBACK_HEIGHT - NAME_LIFT_ABOVE_HEAD);
    }

    #[test]
    fn posed_frame_replaces_wholesale() {
        let mut frame = NameplateBodyPose::default();
        frame.replace([(3, NameplateSkullPose::MissingBodyOrSkull)]);
        assert!(frame.get(3).is_some());
        frame.replace([(4, NameplateSkullPose::TrueSceneSpace([0.0; 3]))]);
        assert!(frame.get(3).is_none());
        assert!(frame.get(4).is_some());
    }

    #[test]
    fn ray_straight_at_a_player_hits_the_near_face() {
        let origin = Vec3::new(200.0, 0.0, 0.0);
        let head = Vec3::new(200.0, 0.0, 70.0);
        let hit = crosshair_hit_distance(EYE, Vec3::X, origin, head).unwrap();
        assert!((hit - 185.0).abs() < 1e-3, "{hit}");
    }

    #[test]
    fn ray_off_to_the_side_misses() {
        let origin = Vec3::new(200.0, 100.0, 0.0);
        let head = Vec3::new(200.0, 100.0, 70.0);
        assert_eq!(crosshair_hit_distance(EYE, Vec3::X, origin, head), None);
        // And one pointing away misses too.
        assert_eq!(crosshair_hit_distance(EYE, -Vec3::X, origin, head), None);
    }

    #[test]
    fn ray_parallel_to_a_slab_needs_to_be_inside_it() {
        let origin = Vec3::new(200.0, 0.0, 0.0);
        let head = Vec3::new(200.0, 0.0, 70.0);
        let above = Vec3::new(0.0, 0.0, 200.0);
        assert_eq!(crosshair_hit_distance(above, Vec3::X, origin, head), None);
    }

    #[test]
    fn nearest_crosshair_nominee_wins() {
        let nominee = |ent, crosshair| Nominee {
            ent,
            team: 1,
            shown_name: String::new(),
            anchor: Vec3::ZERO,
            pixel: Vec2::ZERO,
            in_line: true,
            crosshair,
        };
        assert_eq!(under_crosshair(&[]), None);
        assert_eq!(under_crosshair(&[nominee(1, None)]), None);
        let crowd = [nominee(1, Some(300.0)), nominee(2, None), nominee(3, Some(120.0))];
        assert_eq!(under_crosshair(&crowd), Some(3));
    }

    #[test]
    fn a_friend_shows_at_once_and_fades_after_losing_the_line() {
        let mut book = NameBook::default();
        let first = book.step(7, true, true, 1000).expect("friendly names need no fade-in");
        assert!(first > 0.0);
        assert!(book.step(7, true, false, 1000 + FRIENDLY_FADE_OUT_MS / 2).is_some());
        assert_eq!(book.step(7, true, false, 1000 + FRIENDLY_FADE_OUT_MS), None);
        assert!(book.up.is_empty());
    }

    #[test]
    fn an_unearned_stranger_is_never_up() {
        let mut book = NameBook::default();
        assert_eq!(book.step(9, false, false, 500), None);
        assert!(book.up.is_empty());
    }

    #[test]
    fn switching_side_restarts_or_drops_the_name() {
        let mut book = NameBook::default();
        book.step(5, true, true, 100);
        // Earned again as an enemy: a fresh showing.
        book.step(5, false, true, 200);
        assert_eq!(book.up[&5], Showing::starting(200, false));
        // Not earned and the side flipped back: forgotten.
        assert_eq!(book.step(5, true, false, 210), None);
        assert!(!book.up.contains_key(&5));
    }

    #[test]
    fn the_book_empties_when_the_clock_goes_back() {
        let mut book = NameBook::default();
        book.tick(1000);
        book.up.insert(1, Showing::starting(900, true));
        book.tick(1010);
        assert_eq!(book.up.len(), 1);
        book.tick(50);
        assert!(book.up.is_empty());
    }

    #[test]
    fn keep_only_forgets_the_departed() {
        let mut book = NameBook::default();
        book.up.insert(1, Showing::starting(0, true));
        book.up.insert(2, Showing::starting(0, true));
        book.keep_only(|ent| ent == 2);
        assert_eq!(book.up.keys().copied().collect::<Vec<_>>(), vec![2]);
    }
}
