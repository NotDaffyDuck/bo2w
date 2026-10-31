//! BO2's bullet penetration: the depth table, the depth surface of a hit and the trace
//! advance past a surface. Facts: weapon-hits-pen-t6.md section 3.

use crate::surface::SURFACE_TYPE_COUNT;

/// Penetrate types: none, then the three sizes `bullet_penetration_mp` names.
pub const PIERCE_CLASS_TOTAL: usize = 4;
/// Surfaces a depth table holds: BO2's `penetrationDepthTable` is 4 types by 32 surfaces.
pub const PEN_TABLE_SURF_COUNT: usize = SURFACE_TYPE_COUNT;

/// Surface and contents bits, and the flesh surface type.
pub const FACE_BLOCKS_BULLETS: u32 = 0x100;
pub const BRUSH_IS_PANE: u32 = 0x10;
pub const FACE_KIND_BODY: u32 = 7;

/// How many surfaces a bullet may pass, normally and with extended penetration.
pub const PIERCE_PASS_LIMIT: usize = 5;
pub const PIERCE_LONG_PASS_LIMIT: usize = 12;

/// How far past a face the forward and reverse traces restart. The reverse trace also ends this far short of the
/// entry point (bo2mp Bullet_FirePenetrate).
pub const STEP_FORECAST_NUDGE: f32 = 0.135;
pub const STEP_BACKCAST_NUDGE: f32 = 0.01;
/// A bullet this close to grazing a world face does not advance through it.
pub const STEP_ON_COSINE_FLOOR: f32 = 0.125;

/// What a rifle bullet keeps for the next victim.
pub const RIFLE_PASS_THROUGH_FACTOR: f32 = 0.5;

/// `perk_bulletPenetrationMultiplier`'s registered default (2.0, weapon-hits-pen-t6.md 3.8):
/// how much deeper a bullet goes with the bullet penetration perk, times the table's depth.
pub const PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT: f32 = 2.0;

/// The thinnest a passed surface counts as.
pub const PIERCE_DEPTH_MINIMUM: f32 = 1.0;

/// The penetration side of one weapon's bullet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PierceCapability {
    pub wall_pierce_class: i32,
    pub wall_pierce_scale: f32,
    pub long_gun_round: bool,
    /// bo2mp: bBulletImpactExplode (exploding bullets never penetrate).
    pub impact_explode: bool,
}

impl PierceCapability {
    /// A bullet that goes through nothing, at the plain depth.
    const SOLID: Self = Self {
        wall_pierce_class: 0,
        wall_pierce_scale: 1.0,
        long_gun_round: false,
        impact_explode: false,
    };

    /// bo2mp: BO2 penetrates with a penetrate type and no exploding bullets; it never
    /// reads bullet_penetrationEnabled.
    pub const fn pen_gate(self) -> bool {
        !self.impact_explode && self.wall_pierce_class != 0
    }
}

impl Default for PierceCapability {
    fn default() -> Self {
        Self::SOLID
    }
}

/// How deep each penetrate type goes into each surface type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletPierceDepthChart {
    by_class: [[f32; PEN_TABLE_SURF_COUNT]; PIERCE_CLASS_TOTAL],
}

impl BulletPierceDepthChart {
    /// Every depth zero.
    pub const fn empty() -> Self {
        Self { by_class: [[0f32; PEN_TABLE_SURF_COUNT]; PIERCE_CLASS_TOTAL] }
    }

    /// Sets one cell; a cell off the table is ignored.
    pub fn put_depth(&mut self, wall_pierce_class: usize, surf_kind: usize, on: f32) {
        let cell = self.by_class.get_mut(wall_pierce_class).and_then(|row| row.get_mut(surf_kind));
        if let Some(cell) = cell {
            *cell = on;
        }
    }

    /// The depth for a penetrate type into a surface type; 0 for type or surface 0, or
    /// either off the table.
    pub fn depth(self, wall_pierce_class: i32, surf_kind: u32) -> f32 {
        let (row, col) = (wall_pierce_class as usize, surf_kind as usize);
        if row == 0 || col == 0 {
            return 0.0;
        }
        self.by_class.get(row).and_then(|r| r.get(col)).copied().unwrap_or(0.0)
    }
}

impl Default for BulletPierceDepthChart {
    fn default() -> Self {
        BulletPierceDepthChart::empty()
    }
}

/// The surface type a hit's depth is read for: none on a no-penetrate face, else the
/// face's own (6 bits in BO2), falling back to the last one when the face has none.
pub fn pierce_face_kind(surface_flags: u32, prev_surf: u32) -> u32 {
    let own = (surface_flags >> 20) & 0x3f;
    match (surface_flags & FACE_BLOCKS_BULLETS != 0, own) {
        (true, _) => 0,
        (false, 0) if prev_surf != 0 => prev_surf,
        (false, own) => own,
    }
}

/// Where the trace goes on from after a hit: `dist` further along `dir` past a world
/// face, measured square to the face; `None` when the bullet meets the face too
/// glancingly. Entities, or no distance, start at the hit itself.
pub fn advance_trace(
    impact_point: [f32; 3],
    surface_normal: [f32; 3],
    dir: [f32; 3],
    struck_world: bool,
    dist: f32,
) -> Option<[f32; 3]> {
    let through_world = struck_world && dist > 0.0;
    if !through_world {
        return Some(impact_point);
    }
    let into_face = -(surface_normal[0] * dir[0] + surface_normal[1] * dir[1] + surface_normal[2] * dir[2]);
    match into_face < STEP_ON_COSINE_FLOOR {
        true => None,
        false => {
            let along = dist / into_face;
            Some(core::array::from_fn(|i| impact_point[i] + along * dir[i]))
        }
    }
}

/// The size prefixes of `bullet_penetration_mp` keys, by penetrate type from 1.
const PEN_SIZES: [&str; 3] = ["small", "medium", "large"];

/// A `bullet_penetration_mp` key ("small_<surface>", "medium_..", "large_..") as
/// (penetrate type, surface) against a surface list (BO2: `T6_SURFACE_TYPE_NAMES`).
pub fn split_pen_key_in(key: &str, surface_names: &[&str]) -> Option<(usize, usize)> {
    let (size, surface) = key.split_once('_')?;
    let pierce_class = 1 + PEN_SIZES.iter().position(|s| *s == size)?;
    let surf = surface_names.iter().position(|name| *name == surface)?;
    Some((pierce_class, surf))
}
