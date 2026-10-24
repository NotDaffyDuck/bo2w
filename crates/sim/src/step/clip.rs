//! What a move bumps into besides the map: the other players' bodies, the brush models (doors,
//! lifts), the solid script movers, the turned blocker boxes and the other vehicles.
//! `ClipBackend` lends all of that, together with the map, to the movement code.

use std::cell::RefCell;

use bevy_ecs::prelude::Resource;
use movement_t6::bridge::{CollisionBackend, GroundTraceInput};
use trace_t6::{Trace, TraceHitType};

use crate::bullet_collision::{LinkedBrushCollisionBrush, PLAYER_MAXS, PLAYER_MINS};
use crate::frame::FrameWorld;
use crate::gentity::{ScriptMoverGentity, apos_from_entity_state, pos_from_entity_state};
use crate::match_state::ClientLifecycle;
use crate::world::{SimBrush, SimClipBsp, SimClipMesh, clip_move_to_bmodels_struck, clip_trace};

/// A box that other players' moves can run into.
pub(super) struct BodyBox {
    pub(super) entnum: u16,
    pub(super) origin: [f32; 3],
    /// Set when the box is not the standing player's: a script mover's own packed `solid` box
    /// (a Black Ops II zombie).
    pub(super) bounds: Option<([f32; 3], [f32; 3])>,
    /// The entity whose own moves pass through this box: a body's own number, or the climber a
    /// landing spot belongs to.
    pub(super) owner: u16,
    /// What the box stops: a body, or (a landing spot) player clip.
    pub(super) contents: u32,
}

impl BodyBox {
    fn of_player(client: u16, origin: [f32; 3]) -> Self {
        Self {
            entnum: client,
            origin,
            bounds: None,
            owner: client,
            contents: crate::world::CONTENTS_BODY,
        }
    }

    /// The box of an entity that is solid as a packed `solid` value: half width in the low byte,
    /// depth below the origin in the next, height above it (offset by 32) in the third.
    fn of_packed_solid(entnum: u16, origin: [f32; 3], solid: u32) -> Self {
        let half = (solid & 0xff) as f32;
        let below = ((solid >> 8) & 0xff) as f32;
        let above = ((solid >> 16) & 0xff) as f32 - 32.0;
        Self {
            entnum,
            origin,
            bounds: Some(([-half, -half, -below], [half, half, above])),
            owner: entnum,
            contents: crate::world::CONTENTS_BODY,
        }
    }

    /// Where a climber will land: the climber's box, in player clip. Black Ops II makes an
    /// entity of each, not a client, so a hit on one names no player (stepping onto it is fine).
    fn of_landing_spot(climber: u16, origin: [f32; 3]) -> Self {
        Self {
            entnum: trace_t6::ENTITYNUM_WORLD,
            origin,
            bounds: None,
            owner: climber,
            contents: crate::world::CONTENTS_PLAYERCLIP,
        }
    }

    fn extent(&self) -> ([f32; 3], [f32; 3]) {
        self.bounds.unwrap_or((PLAYER_MINS, PLAYER_MAXS))
    }
}

/// Every box a move can be stopped by: the living players, the remote bodies the client
/// predicts with, the packed-solid movers (a crowd of zombies corners a player, and the same
/// rows reach his client's prediction in the snapshot) and the climbers' landing spots.
pub(super) fn alive_body_clips(world: &FrameWorld) -> Vec<BodyBox> {
    let mut boxes = Vec::new();
    for id in world.client_ids_sorted() {
        let alive = world
            .client_meta(id)
            .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive);
        if !alive {
            continue;
        }
        if let Some(ps) = world.player(id) {
            boxes.push(BodyBox::of_player(id.0 as u16, ps.origin));
        }
    }
    for remote in world.prediction_remote_bodies() {
        boxes.push(BodyBox::of_player(remote.client.0 as u16, remote.origin));
    }
    for (entnum, origin, solid) in world.packed_solid_movers() {
        boxes.push(BodyBox::of_packed_solid(entnum, origin, solid));
    }
    for spot in world.mantle_blockers() {
        boxes.push(BodyBox::of_landing_spot(spot.owner, spot.origin));
    }
    boxes
}

/// A box at a place: its origin and its (mins, maxs).
type PlacedBox = ([f32; 3], [f32; 3], [f32; 3]);

/// Whether two placed boxes overlap, edges touching not counting.
fn boxes_overlap(a: PlacedBox, b: PlacedBox) -> bool {
    let ((a_at, a_lo, a_hi), (b_at, b_lo, b_hi)) = (a, b);
    (0..3).all(|axis| a_at[axis] + a_lo[axis] < b_at[axis] + b_hi[axis] && a_at[axis] + a_hi[axis] > b_at[axis] + b_lo[axis])
}

/// Everything one move is clipped against besides the map itself.
#[derive(Clone, Copy)]
pub(super) struct ClipBackend<'a> {
    pub(super) brushes: &'a [SimBrush],
    pub(super) bsp: &'a SimClipBsp,
    pub(super) mesh: &'a SimClipMesh,
    /// Damage dealt to each glass piece (piece, damage); a broken piece stops nothing.
    pub(super) glass_damage: &'a [(u32, u16)],
    pub(super) bodies: &'a [BodyBox],
    /// The mover's own entity number: its own traces pass through its own boxes.
    pub(super) self_entnum: u16,
    pub(super) cmodels: &'a [asset_world::ClipCmodel],
    /// The brush models (doors, lifts), each with its entity number.
    pub(super) linked_brushes: &'a [(u16, LinkedBrushCollisionBrush)],
    /// Who owns each linked brush (same order as `linked_brushes`).
    pub(super) linked_owners: &'a [crate::AuthorityModelOwner],
    /// The traces that ended on a linked brush (fraction, end, its index); `None` where nobody
    /// reads which models a move touched (vehicles, benches).
    pub(super) struck: Option<&'a RefCell<Vec<(f32, [f32; 3], usize)>>>,
    /// The owners of the models the move touched (BO2's pmove keeps 32 at most).
    pub(super) touched: Option<&'a RefCell<Vec<crate::AuthorityModelOwner>>>,
    /// Solid collision models as turned boxes (entity number, brush).
    pub(super) blockers: &'a [(u16, SimBrush)],
    /// The script movers by entity number, for the platforms players ride
    /// ([`movement_t6::Bo2Collision::mover_pose`]).
    pub(super) movers: &'a [ScriptMoverGentity],
    /// The other scorestreak vehicles (only a vehicle's own move sees them).
    pub(super) vehicles: &'a [OtherVehicle],
    /// Black Ops II's own collision is on (a Black Ops II game always uses it).
    pub(super) gjk: bool,
}

impl ClipBackend<'_> {
    fn glass_stands(&self, piece: u16) -> bool {
        let damage = self
            .glass_damage
            .iter()
            .find(|(id, _)| *id == u32::from(piece))
            .map_or(0, |&(_, damage)| damage);
        crate::world_objects::glass_piece_is_solid(damage)
    }
}

/// The script models each player's move touched this tick (BO2's pmove touch list); the map
/// scripts get their "touch" notifies from it.
#[derive(Resource, Default)]
pub(crate) struct PlayerModelTouches(pub(crate) Vec<(u32, crate::AuthorityModelOwner)>);

/// bo2zm M3: the world's solid collision models (Nuketown's patch walls,
/// perk machines, the Mystery Box) as six-plane brushes.
pub(super) fn oriented_blocker_brushes(world: &FrameWorld) -> Vec<(u16, SimBrush)> {
    world
        .oriented_blockers()
        .iter()
        .map(|b| {
            (
                b.entnum,
                SimBrush {
                    planes: b.planes().to_vec(),
                    contents: crate::bullet_collision::CONTENTS_SOLID,
                    plane_surface_flags: vec![0; 6],
                    glass_encoded: 0,
                    verts: Vec::new(),
                    hit_surface_flags: 0,
                },
            )
        })
        .collect()
}

/// bo2zm M3: the move clipped against the turned boxes too. One he is
/// already inside never holds him (he walks out of it), as with entities'
/// boxes.
fn clip_move_to_blockers(
    mut hit: Trace,
    blockers: &[(u16, SimBrush)],
    input: GroundTraceInput,
) -> Trace {
    for (entnum, brush) in blockers {
        let other = crate::world::t6_brushes_trace(
            &[brush],
            input.start,
            input.end,
            input.mins,
            input.maxs,
            input.tracemask,
        );
        if other.startsolid || other.allsolid || other.fraction >= hit.fraction {
            continue;
        }
        hit = Trace {
            hit_type: TraceHitType::Entity,
            hit_id: *entnum,
            ..other
        };
    }
    hit
}

/// A vehicle's move clipped against the other vehicles' turned boxes, as against the blockers
/// (a box it starts inside never holds it).
fn clip_move_to_vehicles(hit: Trace, vehicles: &[OtherVehicle], self_entnum: u16, input: GroundTraceInput) -> Trace {
    if input.tracemask & VEHICLE_CONTENTS == 0 {
        return hit;
    }
    let others: Vec<(u16, SimBrush)> = vehicles
        .iter()
        .filter(|other| other.entnum != self_entnum)
        .map(|other| (other.entnum, other.brush.clone()))
        .collect();
    clip_move_to_blockers(hit, &others, input)
}

/// bo2mp: the world's shapes for Black Ops II's own collision, as its server
/// gathers them for the box a move can reach: the world's brushes and terrain
/// patches, the brush models (doors, movers), the other players' bodies and
/// (a vehicle's move) the other vehicles.
/// Static models are not in it (BO2's query has only brushes and terrain);
/// the bo2zm blockers and glass are not either.
impl movement_t6::Bo2Collision for ClipBackend<'_> {
    fn gjk_on(&self) -> bool {
        self.gjk
    }

    fn gjk_query<'a>(
        &'a self,
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
        out: &mut Vec<movement_t6::gjk::Geom<'a>>,
    ) {
        use movement_t6::gjk::{Geom, IDENTITY, Shape, player_hull};
        let reaches = |lo: [f32; 3], hi: [f32; 3]| (0..3).all(|a| lo[a] <= maxs[a] && mins[a] <= hi[a]);

        // The brushes of the leaves the box reaches (with no map, a test's
        // hand-made room: every brush).
        let mut brush_ids: Vec<u16> = Vec::new();
        match self.bsp.t6 {
            Some(map) => clipmap_t6::box_brushes(map, mins, maxs, mask, &mut brush_ids),
            None => brush_ids.extend((0..self.brushes.len()).filter_map(|i| u16::try_from(i).ok())),
        }
        for &id in &brush_ids {
            let Some(brush) = self.brushes.get(usize::from(id)) else {
                continue;
            };
            if brush.contents & mask == 0 {
                continue;
            }
            if let Some(geom) = brush_geom(brush, 1 << 32 | u64::from(id))
                && reaches(geom.mins, geom.maxs)
            {
                out.push(geom);
            }
        }

        // Terrain: each tree of those leaves the box reaches, down to its
        // patches.
        if let Some(map) = self.bsp.t6 {
            let mut leafs = [0u16; 1024];
            let mut ll = clipmap_t6::LeafList::new(&mut leafs, mins, maxs);
            clipmap_t6::box_leafnums(map, &mut ll, 0);
            let mut roots: Vec<usize> = Vec::new();
            for &l in ll.leafs() {
                let Some(leaf) = map.leafs.get(usize::from(l)) else {
                    continue;
                };
                let first = usize::from(leaf.first_coll_aabb);
                for root in first..first + usize::from(leaf.coll_aabb_count) {
                    if !roots.contains(&root) {
                        roots.push(root);
                    }
                }
            }
            let mut stack: Vec<usize> = roots.into_iter().rev().collect();
            while let Some(index) = stack.pop() {
                let Some(node) = map.aabb_trees.get(index) else {
                    continue;
                };
                let lo = [0, 1, 2].map(|a| node.origin[a] - node.half_size[a]);
                let hi = [0, 1, 2].map(|a| node.origin[a] + node.half_size[a]);
                if !reaches(lo, hi) {
                    continue;
                }
                let u = node.u as usize;
                if node.child_count != 0 {
                    stack.extend((u..u + usize::from(node.child_count)).rev());
                    continue;
                }
                let Some(part) = map.partitions.get(u) else {
                    continue;
                };
                let Ok(first) = usize::try_from(part.first_tri) else {
                    continue;
                };
                let Some(tris) = map.tri_indices.get(first..first + usize::from(part.tri_count)) else {
                    continue;
                };
                // BO2 asks the tree's material.
                let material = map.materials.get(usize::from(node.material_index)).copied().unwrap_or_default();
                if material.content_flags & mask == 0 {
                    continue;
                }
                out.push(Geom {
                    shape: Shape::Partition { verts: map.verts, indices: tris.as_flattened() },
                    axis: IDENTITY,
                    origin: [0.0; 3],
                    mins: lo,
                    maxs: hi,
                    contents: material.content_flags,
                    surface_flags: asset_world::t5_surface_flags(material.surface_flags),
                    hit_id: trace_t6::ENTITYNUM_WORLD,
                    sentient: false,
                    no_push_out: false,
                    key: 2 << 32 | u as u64,
                });
            }
        }

        // Brush models, turned and moved as their entity is.
        for (n, (entnum, linked)) in self.linked_brushes.iter().enumerate() {
            let Some(cmodel) = self.cmodels.get(linked.cmodel_handle as usize) else {
                continue;
            };
            let reach = cmodel.radius + 1.0;
            if !reaches(linked.origin.map(|o| o - reach), linked.origin.map(|o| o + reach)) {
                continue;
            }
            let axis = game_math::angles_to_axis(linked.angles);
            let first = cmodel.first_brush as usize;
            let ids = self
                .bsp
                .leafbrushes
                .get(first..first + usize::from(cmodel.num_brushes))
                .unwrap_or(&[]);
            for &id in ids {
                let Some(brush) = self.brushes.get(usize::from(id)) else {
                    continue;
                };
                if brush.contents & mask == 0 {
                    continue;
                }
                let Some(mut geom) = brush_geom(brush, 3 << 32 | (n as u64) << 16 | u64::from(id)) else {
                    continue;
                };
                (geom.mins, geom.maxs) = turned_box(geom.mins, geom.maxs, &axis, linked.origin);
                geom.axis = axis;
                geom.origin = linked.origin;
                geom.hit_id = *entnum;
                if reaches(geom.mins, geom.maxs) {
                    out.push(geom);
                }
            }
        }

        // The other players' bodies (and the bo2zm zombies' boxes, and the
        // climbers' landing spots).
        {
            for (n, body) in self.bodies.iter().enumerate() {
                if body.owner == self.self_entnum || mask & body.contents == 0 {
                    continue;
                }
                let (body_mins, body_maxs) = body.bounds.unwrap_or((PLAYER_MINS, PLAYER_MAXS));
                let hull = player_hull(body_mins, body_maxs, 0.0);
                let (lo, hi) = hull.calc_aabb();
                let geom = Geom {
                    shape: Shape::Player { hull },
                    axis: IDENTITY,
                    origin: body.origin,
                    mins: [0, 1, 2].map(|a| body.origin[a] + lo[a]),
                    maxs: [0, 1, 2].map(|a| body.origin[a] + hi[a]),
                    contents: body.contents,
                    surface_flags: 0,
                    hit_id: body.entnum,
                    sentient: body.contents == crate::world::CONTENTS_BODY,
                    no_push_out: true,
                    key: 4 << 32 | n as u64,
                };
                if reaches(geom.mins, geom.maxs) {
                    out.push(geom);
                }
            }
        }

        // bo2mp: the other vehicles, each its model's collision piece turned
        // to its angles at its origin (never the vehicle moving).
        if mask & VEHICLE_CONTENTS != 0 {
            for (n, v) in self.vehicles.iter().enumerate() {
                let (lo, hi) = v.world;
                if v.entnum == self.self_entnum || !reaches(lo, hi) {
                    continue;
                }
                out.push(Geom {
                    shape: Shape::Brush { verts: &v.verts, mins: v.mins, maxs: v.maxs, sides: &[] },
                    axis: v.axis,
                    origin: v.origin,
                    mins: lo,
                    maxs: hi,
                    contents: VEHICLE_CONTENTS,
                    surface_flags: 0,
                    hit_id: v.entnum,
                    sentient: false,
                    no_push_out: v.no_push_out,
                    key: 5 << 32 | n as u64,
                });
            }
        }
    }

    /// CM_GetWaterHeight: a point traced down through the feet, from 200
    /// above them to 200 below, that only water stops.
    fn water_height(&self, origin: [f32; 3]) -> f32 {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER, WATER_SEARCH};
        let start = [origin[0], origin[1], origin[2] + WATER_SEARCH];
        let end = [origin[0], origin[1], origin[2] - WATER_SEARCH];
        let hit = clip_trace(
            self.brushes,
            self.bsp,
            self.mesh,
            start,
            end,
            [0.0; 3],
            [0.0; 3],
            CONTENTS_WATER,
            &|_| true,
        );
        if hit.fraction < 1.0 {
            start[2] + (end[2] - start[2]) * hit.fraction
        } else {
            NO_WATER
        }
    }

    /// A script mover's pose on its path at `time`: where it was at the
    /// last command and is at this one (BG_EvaluateTrajectory on its
    /// position and its angles).
    fn mover_pose(&self, entnum: i32, time: i32) -> Option<([f32; 3], [f32; 3])> {
        let i = self
            .movers
            .binary_search_by_key(&entnum, |mover| mover.state.number)
            .ok()?;
        let state = &self.movers[i].state;
        Some((
            entity_t6::evaluate_trajectory(&pos_from_entity_state(state), time),
            entity_t6::evaluate_trajectory(&apos_from_entity_state(state), time),
        ))
    }
}

/// bo2mp: one brush as Black Ops II's collision sees it, in its own frame: a
/// box when it has only its six axial sides, else its corner points and its
/// other sides. None: a brush with sides but no corner points (not a Black
/// Ops II map's).
fn brush_geom(brush: &SimBrush, key: u64) -> Option<movement_t6::gjk::Geom<'_>> {
    use movement_t6::gjk::{Geom, IDENTITY, Shape};
    let p = brush.planes.get(..6)?;
    let mins = [-p[1][3], -p[3][3], -p[5][3]];
    let maxs = [p[0][3], p[2][3], p[4][3]];
    let shape = if brush.planes.len() == 6 {
        Shape::Box {
            center: [0, 1, 2].map(|a| (mins[a] + maxs[a]) * 0.5),
            half: [0, 1, 2].map(|a| (maxs[a] - mins[a]) * 0.5),
            brush: true,
        }
    } else if brush.verts.is_empty() {
        return None;
    } else {
        Shape::Brush { verts: &brush.verts, mins, maxs, sides: &brush.planes[6..] }
    };
    Some(Geom {
        shape,
        axis: IDENTITY,
        origin: [0.0; 3],
        mins,
        maxs,
        contents: brush.contents,
        surface_flags: brush.hit_surface_flags,
        hit_id: trace_t6::ENTITYNUM_WORLD,
        sentient: false,
        no_push_out: false,
        key,
    })
}

/// bo2mp: the world box around a local box turned by `axis` (rows) and moved
/// to `origin`.
fn turned_box(
    mins: [f32; 3],
    maxs: [f32; 3],
    axis: &[[f32; 3]; 3],
    origin: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let mut lo = origin;
    let mut hi = origin;
    for a in 0..3 {
        for row in 0..3 {
            let (x, y) = (axis[row][a] * mins[row], axis[row][a] * maxs[row]);
            lo[a] += x.min(y);
            hi[a] += x.max(y);
        }
    }
    (lo, hi)
}

/// bo2mp: a map's collision as the player's moves see it, with nobody else
/// in it, for headless benches (`t6walk`). `gjk` picks Black Ops II's own
/// collision or the box traces.
#[doc(hidden)]
pub fn bo2_map_collision<'a>(
    brushes: &'a [SimBrush],
    bsp: &'a SimClipBsp,
    mesh: &'a SimClipMesh,
    gjk: bool,
) -> impl movement_t6::Bo2Collision + 'a {
    ClipBackend {
        brushes,
        bsp,
        mesh,
        glass_damage: &[],
        bodies: &[],
        self_entnum: 0,
        cmodels: &[],
        linked_brushes: &[],
        linked_owners: &[],
        struck: None,
        touched: None,
        blockers: &[],
        movers: &[],
        vehicles: &[],
        gjk,
    }
}

/// bo2mp: a vehicle's contents (BO2's `VEH_InitEntity`): what a vehicle's
/// own clip mask meets of another.
const VEHICLE_CONTENTS: u32 = 0x0081_2080;

/// bo2mp: another scorestreak vehicle as a vehicle's move meets it. BO2's
/// collision query adds a vehicle by its model's collision pieces, turned to
/// its angles at its origin; one whose model has none is not in it, and a
/// hidden one has no contents.
#[derive(Clone)]
pub(crate) struct OtherVehicle {
    pub(super) entnum: u16,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    /// Its collision piece's corner points and their box, in its own frame.
    verts: Vec<[f32; 3]>,
    mins: [f32; 3],
    maxs: [f32; 3],
    /// That box turned and placed: its world box, and as a brush (with the
    /// world box's sides) for the box traces.
    world: ([f32; 3], [f32; 3]),
    brush: SimBrush,
    /// It is driven from afar (its def's `remoteControl`): BO2's move is
    /// never pushed out of one, as out of a player.
    no_push_out: bool,
}

impl OtherVehicle {
    /// None: no collision piece's corner points.
    pub(crate) fn new(
        entnum: u16,
        origin: [f32; 3],
        angles: [f32; 3],
        verts: Vec<[f32; 3]>,
        no_push_out: bool,
    ) -> Option<Self> {
        let first = *verts.first()?;
        let (mut mins, mut maxs) = (first, first);
        for p in &verts {
            for a in 0..3 {
                mins[a] = mins[a].min(p[a]);
                maxs[a] = maxs[a].max(p[a]);
            }
        }
        let axis = game_math::angles_to_axis(angles);
        let (lo, hi) = turned_box(mins, maxs, &axis, origin);
        let mut planes = vec![
            [1.0, 0.0, 0.0, hi[0]],
            [-1.0, 0.0, 0.0, -lo[0]],
            [0.0, 1.0, 0.0, hi[1]],
            [0.0, -1.0, 0.0, -lo[1]],
            [0.0, 0.0, 1.0, hi[2]],
            [0.0, 0.0, -1.0, -lo[2]],
        ];
        for (row, n) in axis.iter().enumerate() {
            // A side along the world's axes is the world box's own (the box
            // traces push a turned side out less, as a capsule's).
            if n.iter().any(|c| c.abs() > 0.9999) {
                continue;
            }
            let d = n[0] * origin[0] + n[1] * origin[1] + n[2] * origin[2];
            planes.push([n[0], n[1], n[2], d + maxs[row]]);
            planes.push([-n[0], -n[1], -n[2], -(d + mins[row])]);
        }
        let brush = SimBrush {
            plane_surface_flags: vec![0; planes.len()],
            planes,
            contents: VEHICLE_CONTENTS,
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: 0,
        };
        Some(Self {
            entnum,
            origin,
            axis,
            verts,
            mins,
            maxs,
            world: (lo, hi),
            brush,
            no_push_out,
        })
    }
}

/// bo2mp: what a scorestreak vehicle's move meets besides the map, as a
/// player's move meets them (BO2's vehicle clip mask has bodies and
/// movers): the players' bodies, the brush models (doors, lifts), the
/// solid script movers, the bo2zm blockers and the other vehicles
/// ([`VehicleClip::with_vehicles`]), never the vehicle itself.
/// Gathered once for a move; [`VehicleClip::collision`] lends it with the
/// map.
pub struct VehicleClip {
    pub(super) bodies: Vec<BodyBox>,
    pub(super) linked_brushes: Vec<(u16, LinkedBrushCollisionBrush)>,
    pub(super) blockers: Vec<(u16, SimBrush)>,
    pub(super) movers: Vec<ScriptMoverGentity>,
    pub(super) glass_damage: Vec<(u32, u16)>,
    /// The vehicle's own entity number: its own box never stops it.
    pub(super) self_entnum: u16,
    pub(super) vehicles: Vec<OtherVehicle>,
}

impl VehicleClip {
    pub(crate) fn gather(world: &mut FrameWorld, self_entnum: u16) -> Self {
        let movers = crate::frame::collect_script_movers(world.ecs());
        let mover_numbers: std::collections::BTreeMap<_, u16> = movers
            .iter()
            .map(|mover| (mover.id, mover.state.number as u16))
            .collect();
        let linked_brushes = world
            .entity_collision_capabilities()
            .iter()
            .flat_map(|c| {
                let entnum = c
                    .owner
                    .script_model()
                    .and_then(|id| mover_numbers.get(&id).copied())
                    .unwrap_or(trace_t6::ENTITYNUM_WORLD);
                c.solid_brushes().iter().map(move |b| (entnum, b.clone()))
            })
            .filter(|(entnum, _)| *entnum != self_entnum)
            .collect();
        let mut blockers = oriented_blocker_brushes(world);
        blockers.retain(|(entnum, _)| *entnum != self_entnum);
        Self {
            bodies: alive_body_clips(world),
            linked_brushes,
            blockers,
            movers,
            glass_damage: world.world_objects().glass_damage_pairs(),
            self_entnum,
            vehicles: Vec::new(),
        }
    }

    /// The other scorestreak vehicles too (its own is left out).
    pub(crate) fn with_vehicles(mut self, mut vehicles: Vec<OtherVehicle>) -> Self {
        let own = self.self_entnum;
        vehicles.retain(|v| v.entnum != own);
        self.vehicles = vehicles;
        self
    }

    /// The map in `content` with what was gathered; `gjk` picks Black Ops
    /// II's own collision or the box traces.
    pub fn collision<'a>(&'a self, content: &'a crate::world::SimContent, gjk: bool) -> impl movement_t6::Bo2Collision + 'a {
        self.backend(
            content.clip_brushes(),
            content.clip_bsp(),
            content.clip_mesh(),
            &content.clip_cmodels().models,
            gjk,
        )
    }

    pub(super) fn backend<'a>(
        &'a self,
        brushes: &'a [SimBrush],
        bsp: &'a SimClipBsp,
        mesh: &'a SimClipMesh,
        cmodels: &'a [asset_world::ClipCmodel],
        gjk: bool,
    ) -> ClipBackend<'a> {
        ClipBackend {
            brushes,
            bsp,
            mesh,
            glass_damage: &self.glass_damage,
            bodies: &self.bodies,
            self_entnum: self.self_entnum,
            cmodels,
            linked_brushes: &self.linked_brushes,
            // A vehicle's move reports no touches.
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &self.blockers,
            movers: &self.movers,
            vehicles: &self.vehicles,
            gjk,
        }
    }
}

impl CollisionBackend for ClipBackend<'_> {
    fn trace(&self, input: GroundTraceInput) -> trace_t6::Trace {
        let map_hit = clip_trace(
            self.brushes,
            self.bsp,
            self.mesh,
            input.start,
            input.end,
            input.mins,
            input.maxs,
            input.tracemask,
            &|piece| self.glass_stands(piece),
        );
        let (after_models, struck_model) = clip_move_to_bmodels_struck(
            map_hit,
            self.bsp.t6,
            self.linked_brushes.iter().map(|(entnum, brush)| (*entnum, brush)),
            input,
        );

        let mut hit = clip_move_to_players(after_models, self.bsp.t6, self.bodies, self.self_entnum, input);
        hit = clip_move_to_blockers(hit, self.blockers, input);
        hit = clip_move_to_vehicles(hit, self.vehicles, self.self_entnum, input);

        // Note the brush model that stopped the move, when nothing nearer replaced it.
        let model_decided = hit.fraction == after_models.fraction
            && hit.hit_type == after_models.hit_type
            && hit.hit_id == after_models.hit_id;
        if let (Some(index), Some(log), true) = (struck_model, self.struck, model_decided) {
            log.borrow_mut()
                .push((hit.fraction, hit.end_pos(input.start, input.end), index));
        }
        hit
    }

    fn touch_entity(&self, trace: &trace_t6::Trace, end_pos: [f32; 3]) {
        let (Some(log), Some(touched)) = (self.struck, self.touched) else {
            return;
        };
        let index = log
            .borrow()
            .iter()
            .rev()
            .find(|(fraction, end, _)| *fraction == trace.fraction && *end == end_pos)
            .map(|&(_, _, index)| index);
        let Some(owner) = index.and_then(|index| self.linked_owners.get(index).copied()) else {
            return;
        };
        let mut list = touched.borrow_mut();
        // BO2's PM_AddTouchEnt: each owner once, 32 at most.
        if list.len() < 32 && !list.contains(&owner) {
            list.push(owner);
        }
    }
}

/// Inside an entity's own box (a zombie stepped into the player as the player stepped into it)
/// the player may move away from its centre, never deeper: deeper stops at once, on a plane
/// facing away from the centre.
fn stop_deeper_in_box(hit: &mut Trace, body: &BodyBox, input: &GroundTraceInput) -> bool {
    let away = |p: [f32; 3]| (p[0] - body.origin[0]).powi(2) + (p[1] - body.origin[1]).powi(2);
    if away(input.end) >= away(input.start) {
        return false;
    }
    let out = [input.start[0] - body.origin[0], input.start[1] - body.origin[1]];
    let length = (out[0] * out[0] + out[1] * out[1]).sqrt();
    hit.fraction = 0.0;
    hit.normal = if length > 1e-3 {
        [out[0] / length, out[1] / length, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    hit.cflags = body.contents;
    hit.hit_type = TraceHitType::Entity;
    hit.hit_id = body.entnum;
    hit.walkable = false;
    true
}

/// The move clipped against the other players' bodies. On a Black Ops II map each body is its
/// temporary box (`CM_TempBoxModel`), traced as a capsule.
fn clip_move_to_players(
    mut hit: Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    bodies: &[BodyBox],
    self_entnum: u16,
    input: GroundTraceInput,
) -> Trace {
    if hit.fraction == 0.0 {
        return hit;
    }
    for body in bodies.iter().filter(|body| body.owner != self_entnum) {
        let (body_mins, body_maxs) = body.extent();
        let inside_packed_box = body.bounds.is_some()
            && boxes_overlap(
                (input.start, input.mins, input.maxs),
                (body.origin, body_mins, body_maxs),
            );
        if inside_packed_box {
            if stop_deeper_in_box(&mut hit, body, &input) {
                return hit;
            }
            continue;
        }

        // With no map (no brushes at all) the temporary box needs none.
        let empty_map = clipmap_t6::ClipMap::default();
        let other = crate::world::t6_transformed_trace(
            t6.unwrap_or(&empty_map),
            input.start,
            input.end,
            input.mins,
            input.maxs,
            crate::world::T6Model::TempBox {
                mins: body_mins,
                maxs: body_maxs,
                contents: body.contents,
            },
            input.tracemask,
            body.origin,
            [0.0; 3],
        );
        let nearer = other.fraction < hit.fraction;
        if !nearer && !other.startsolid && !other.allsolid {
            continue;
        }
        let (started_solid, all_solid) = (hit.startsolid | other.startsolid, hit.allsolid | other.allsolid);
        if nearer {
            hit = other;
            hit.hit_type = TraceHitType::Entity;
            hit.hit_id = body.entnum;
        }
        hit.startsolid = started_solid;
        hit.allsolid = all_solid;
    }
    hit
}
