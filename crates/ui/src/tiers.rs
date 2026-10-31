//! Screens are stacked as tiers: the in-game HUD, debug text, the menu shell,
//! the load screen and a top overlay. One resource says which tiers are up,
//! and a PostUpdate system copies that onto every root tagged with an
//! `OverlayTier`.

use std::time::Instant;

use bevy::ecs::query::QueryData;
use bevy::prelude::*;
use bevy::text::FontSize;

const OXANIUM_TTF: &[u8] = include_bytes!("../assets/Oxanium-Regular.ttf");

/// Handle to the face all on-screen text uses.
#[derive(Resource, Clone, Debug)]
pub struct MenuTypeface(pub Handle<Font>);

pub(crate) struct MenuTypefaceModule;

impl Plugin for MenuTypefaceModule {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, MenuTypeface::load);
    }
}

impl MenuTypeface {
    /// Files the bundled face with the fonts and keeps its handle.
    fn load(mut cmd: Commands, mut faces: ResMut<Assets<Font>>) {
        let face = Font::from_bytes(OXANIUM_TTF.to_vec());
        cmd.insert_resource(Self(faces.add(face)));
    }
}

/// A `TextFont` in the menu face, `points` pixels tall.
pub fn match_typeface(face: &Handle<Font>, points: f32) -> TextFont {
    let mut style = TextFont::default();
    style.font = face.clone().into();
    style.font_size = FontSize::Px(points);
    style
}

/// The tier a root node or camera sits in.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayTier {
    Hud,
    Debug,
    Shell,
    Loading,
    Overlay,
}

impl OverlayTier {
    const TIERS: usize = 5;

    const fn index(self) -> usize {
        self as usize
    }
}

/// On a tier root, also drive `Visibility` from the tier, not only
/// `Node.display`.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct OverlayStackShown;

pub use frame::MenuPaintPass;
pub use frame::UiCamera;

/// The tiers that are up right now.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayTierSet {
    shown: [bool; OverlayTier::TIERS],
}

impl OverlayTierSet {
    /// Raises exactly the given tiers. Leaves the set untouched (so change
    /// detection stays quiet) when nothing would differ.
    pub fn raise_exactly(&mut self, picked: impl IntoIterator<Item = OverlayTier>) {
        let mut next = [false; OverlayTier::TIERS];
        for tier in picked {
            next[tier.index()] = true;
        }
        if next != self.shown {
            self.shown = next;
        }
    }

    pub fn is_raised(&self, tier: OverlayTier) -> bool {
        matches!(self.shown.get(tier.index()), Some(true))
    }
}

/// Tier systems run once the screen logic has picked its tiers, ahead of
/// UI layout and visibility propagation.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayTierPass;

/// Time spent on tier upkeep in the last frame.
#[derive(Resource, Debug, Default, Clone)]
pub struct OverlayTierCpuSide {
    pub last_ms: Option<f32>,
    began: Option<Instant>,
}

fn start_layer_clock(mut cost: ResMut<OverlayTierCpuSide>) {
    cost.began = Some(Instant::now());
}

fn stop_layer_clock(mut cost: ResMut<OverlayTierCpuSide>) {
    if let Some(t0) = cost.began {
        let secs = t0.elapsed().as_secs_f32();
        cost.last_ms = Some(secs * 1000.0);
    }
}

pub(crate) struct OverlayTierModule;

impl Plugin for OverlayTierModule {
    fn build(&self, app: &mut App) {
        let layout_gate = bevy::ui::UiSystems::Prepare;
        let propagate = bevy::camera::visibility::VisibilitySystems::VisibilityPropagate;
        app.init_resource::<OverlayTierSet>();
        app.init_resource::<MenuPaintPass>();
        app.init_resource::<OverlayTierCpuSide>();
        let pass = OverlayTierPass.before(layout_gate.clone()).before(propagate);
        app.configure_sets(PostUpdate, pass);
        let clock_on = start_layer_clock.before(crate::screen_tiers::refresh_overlay_tiers);
        app.add_systems(PostUpdate, clock_on);
        app.add_systems(PostUpdate, enact_overlay_tiers.in_set(OverlayTierPass));
        let tail = (
            stop_layer_clock.after(enact_overlay_tiers),
            pick_ui_camera.after(OverlayTierPass),
        );
        app.add_systems(PostUpdate, tail.before(layout_gate));
    }
}

/// Everything a tier root may carry.
#[derive(QueryData)]
#[query_data(mutable)]
struct TierRoot {
    camera: Option<&'static mut Camera>,
    tier: &'static OverlayTier,
    follows: Has<OverlayStackShown>,
    shown: Option<&'static mut Visibility>,
    node: Option<&'static mut Node>,
}

/// Pushes the tier state onto each root: node display, visibility when the
/// root asks for it, and whether its camera is on. Writes only on change.
fn enact_overlay_tiers(mut anchors: Query<TierRoot>, layers: Res<OverlayTierSet>) {
    for root in anchors.iter_mut() {
        let on = layers.is_raised(*root.tier);
        let display = if on { Display::Flex } else { Display::None };
        let state = if on { Visibility::Inherited } else { Visibility::Hidden };
        let follows = root.follows;

        if let Some(mut stale) = root.node.filter(|n| n.display != display) {
            stale.display = display;
        }
        if let Some(mut stale) = root.shown.filter(|v| follows && **v != state) {
            *stale = state;
        }
        if let Some(mut stale) = root.camera.filter(|c| c.is_active != on) {
            stale.is_active = on;
        }
    }
}

/// UI nodes attach by default to the active UI camera drawn last: highest
/// order first, then the newest entity.
fn pick_ui_camera(
    mut cmd: Commands,
    cameras: Query<(Entity, &Camera, Has<IsDefaultUiCamera>), With<UiCamera>>,
) {
    let top = cameras
        .iter()
        .filter(|row| row.1.is_active)
        .max_by_key(|row| (row.1.order, row.0.to_bits()))
        .map(|(id, _, _)| id);

    for (id, _, already) in &cameras {
        let wanted = top == Some(id);
        if wanted && !already {
            cmd.entity(id).insert(IsDefaultUiCamera);
        }
        if already && !wanted {
            cmd.entity(id).remove::<IsDefaultUiCamera>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiers_up(picked: &[OverlayTier]) -> OverlayTierSet {
        let mut set = OverlayTierSet::default();
        set.raise_exactly(picked.iter().copied());
        set
    }

    #[test]
    fn roots_follow_the_layers_that_are_showing() {
        let mut host = App::default();
        host.insert_resource(tiers_up(&[OverlayTier::Hud]));
        host.add_systems(Update, enact_overlay_tiers);
        let mut root = |tier: OverlayTier, start: Visibility, follows: bool| {
            let mut made = host.world_mut().spawn((tier, Node::default(), start));
            if follows {
                made.insert(OverlayStackShown);
            }
            made.id()
        };
        // Each root: its layer, how it starts, whether it follows the stack, then
        // the layout (when it follows) and visibility it ends up with.
        let cases = [
            (OverlayTier::Hud, Visibility::Hidden, true, Some(Display::Flex), Visibility::Inherited),
            (OverlayTier::Shell, Visibility::Inherited, true, Some(Display::None), Visibility::Hidden),
            (OverlayTier::Shell, Visibility::Inherited, false, None, Visibility::Inherited),
        ];
        let made: Vec<Entity> = cases.iter().map(|&(tier, start, follows, ..)| root(tier, start, follows)).collect();
        host.update();
        let world = host.world();
        for (&(.., laid_out, shown), &e) in cases.iter().zip(&made) {
            if let Some(laid_out) = laid_out {
                assert_eq!(world.get::<Node>(e).unwrap().display, laid_out);
            }
            assert_eq!(*world.get::<Visibility>(e).unwrap(), shown);
        }
    }

    #[test]
    fn show_only_marks_just_the_named_layers() {
        let set = tiers_up(&[OverlayTier::Overlay, OverlayTier::Debug]);
        let order = [OverlayTier::Hud, OverlayTier::Debug, OverlayTier::Shell, OverlayTier::Loading, OverlayTier::Overlay];
        let up = order.map(|t| set.is_raised(t));
        assert_eq!(up, [false, true, false, false, true]);
    }
}
