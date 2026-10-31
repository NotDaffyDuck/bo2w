//! bo2zm: a Black Ops II world's colour table (`GfxWorld::lutMaterial`) as
//! the game picks it each frame: the table of the first colour table volume
//! the camera stands in (`GfxWorld::lutVolumes`), table 0 outside them all,
//! or the one a client script forces (`setlutscriptindex`). Each table keeps
//! a weight that rises toward the current table at the volume's transition
//! rate; the two last tables are drawn blended by those weights.

use crate::prepare::scene::camera::FirstPersonLens;
use bevy::prelude::*;

use crate::prepare::scene::world::LevelStage;

pub(crate) fn install(client: &mut App) {
    client.add_systems(Update, apply_t6_lut.in_set(frame::DrawPassStage::Anim));
}

/// Tables a world can hold (`lutIndex` is five bits).
const TABLES: usize = 32;
/// Weight per millisecond when a volume names no transition time.
const INSTANT_RATE: f32 = 0.256;

/// The first volume holding `pos` in bank `banks`: inside is where every
/// plane's `dot(normal, pos) + dist` is <= 0 (as for exposure volumes).
pub fn t6_lut_volume_at(
    volumes: &[asset_world::T6LutVolume],
    pos: [f32; 3],
    banks: u32,
) -> Option<usize> {
    volumes.iter().position(|v| {
        v.banks & banks != 0
            && !v.side_planes.is_empty()
            && v.side_planes
                .iter()
                .all(|p| p[0] * pos[0] + p[1] * pos[1] + p[2] * pos[2] + p[3] <= 0.0)
    })
}

/// Weight per millisecond for a volume's `lutTransitionTime`: none (0) is
/// at once; under 1 it is read as a rate already, else as seconds.
fn rate(time: f32) -> f32 {
    if !(time > 0.0) {
        INSTANT_RATE
    } else if time < 1.0 {
        time
    } else {
        0.001 / time
    }
}


/// One colour table on screen and the volume that chose it.
#[derive(Clone, Copy, Default)]
struct ShownTable {
    table: u32,
    volume: Option<usize>,
}

struct LutState {
    blend: [f32; TABLES],
    /// The tables drawn (previous, current) and the volume behind each.
    prev: ShownTable,
    cur: ShownTable,
    started_at: bool,
}

impl Default for LutState {
    fn default() -> Self {
        Self {
            blend: [0.0; TABLES],
            prev: ShownTable::default(),
            cur: ShownTable::default(),
            started_at: false,
        }
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn apply_t6_lut(
    time: Res<Time>,
    scene: Option<ResMut<LevelStage>>,
    cam_query: Query<&GlobalTransform, With<FirstPersonLens>>,
    mut state: Local<LutState>,
) {
    let Some(mut scene) = scene else {
        *state = LutState::default();
        return;
    };
    // BO2MP_T6_NO_LUT=1: no colour table (test aid).
    let Some(lut) = scene
        .t6_lut
        .clone()
        .filter(|_| std::env::var_os("BO2MP_T6_NO_LUT").is_none())
    else {
        if scene.t6_lut_pick[3] != 0.0 {
            scene.t6_lut_pick = [0.0, 0.0, 1.0, 0.0];
        }
        *state = LutState::default();
        return;
    };
    let tables = (lut.height / 32).max(1);
    let Some(camera) = cam_query.iter().next() else {
        return;
    };
    let pos = camera.translation().to_array();
    let volume = t6_lut_volume_at(&scene.t6_lut_volumes, pos, 1);
    let table = scene
        .t6_lut_script
        .or_else(|| volume.and_then(|i| scene.t6_lut_volumes.get(i)).map(|v| v.table))
        .unwrap_or(0)
        .min(tables - 1);
    if !state.started_at {
        state.started_at = true;
        state.blend = [0.0; TABLES];
        state.blend[table as usize] = 1.0;
        state.cur = ShownTable { table, volume };
        state.prev = state.cur;
    } else if state.cur.table != table {
        diag::info!(
            World,
            "bo2zm colour table {table} (volume {volume:?}) at ({:.0} {:.0} {:.0})",
            pos[0],
            pos[1],
            pos[2]
        );
        state.prev = state.cur;
        state.cur = ShownTable { table, volume };
    } else {
        state.cur.volume = volume;
    }
    let volume_time = |v: Option<usize>| {
        v.and_then(|i| scene.t6_lut_volumes.get(i))
            .map(|v| v.fade_duration)
    };
    let step = time.delta_secs()
        * 1000.0
        * rate(
            volume_time(state.cur.volume)
                .or_else(|| volume_time(state.prev.volume))
                .unwrap_or(0.0),
        );
    state.blend[table as usize] += 2.0 * step;
    for w in state.blend.iter_mut() {
        *w = (*w - step).clamp(0.0, 1.0);
    }
    let (a, b) = (state.prev.table, state.cur.table);
    let (wa, wb) = (state.blend[a as usize], state.blend[b as usize]);
    let w = if a == b || wa + wb <= 0.0 {
        0.0
    } else {
        smoothstep(wa / (wa + wb))
    };
    let pick = [(a * 32) as f32, (b * 32) as f32, w, 1.0];
    if scene.t6_lut_pick != pick {
        scene.t6_lut_pick = pick;
    }
}
