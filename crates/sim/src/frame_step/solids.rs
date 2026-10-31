//! What a move bumps into besides the map: the other players' bodies, the brush models (doors,
//! lifts), the solid script movers, the turned blocker boxes and the other vehicles.
//! `SweepCollisionProvider` lends all of that, together with the map, to the movement code.

use std::cell::RefCell;

use bevy_ecs::prelude::Resource;
use movement_t6::bridge::{SolidProbe, SweepAsk};
use playerstate_t6::game::AvatarState;
use trace_t6::{Trace, TraceHitType};

use crate::shot_trace::{AttachedSolidShape, BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::step_view::StepWorldView;
use crate::ent_slots::{MoverSlot, turn_path_of, move_path_of};
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::{SolidBrush, CollisionTreeData, CollisionTriangleData, clip_move_to_bmodels_struck, sweep_box_trace};

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
    /// A standing player's box at `origin`, named `entnum`, passed through by `owner`'s moves.
    fn standing(entnum: u16, spot: [f32; 3], owner: u16, contents: u32) -> Self {
        Self { entnum, origin: spot, bounds: None, owner, contents }
    }

    fn of_player(client: u16, spot: [f32; 3]) -> Self {
        Self::standing(client, spot, client, crate::sim_parts::SOLID_PLAYER_BODY)
    }

    /// The box of an entity that is solid as a packed `solid` value: half width in the low byte,
    /// depth below the origin in the next, height above it (offset by 32) in the third.
    fn of_packed_solid(entnum: u16, spot: [f32; 3], solid: u32) -> Self {
        let half = (solid & 0xff) as f32;
        let below = ((solid >> 8) & 0xff) as f32;
        let above = ((solid >> 16) & 0xff) as f32 - 32.0;
        let mut packed = Self::standing(entnum, spot, entnum, crate::sim_parts::SOLID_PLAYER_BODY);
        packed.bounds = Some(([-half, -half, -below], [half, half, above]));
        packed
    }

    /// Where a climber will land: the climber's box, in player clip. Black Ops II makes an
    /// entity of each, not a client, so a hit on one names no player (stepping onto it is fine).
    fn of_landing_spot(climber: u16, spot: [f32; 3]) -> Self {
        Self::standing(trace_t6::WORLD_ENTITY_SLOT, spot, climber, crate::sim_parts::CONTENTS_PLAYERCLIP)
    }

    fn extent(&self) -> ([f32; 3], [f32; 3]) {
        self.bounds.unwrap_or((BODY_BOX_LOWER, BODY_BOX_UPPER))
    }
}

/// Every box a move can be stopped by: the living players, the remote bodies the client
/// predicts with, the packed-solid movers (a crowd of zombies corners a player, and the same
/// rows reach his client's prediction in the snapshot) and the climbers' landing spots.
pub(super) fn alive_body_clips(level: &StepWorldView) -> Vec<BodyBox> {
    let mut boxes = Vec::new();
    for seat in level.ordered_player_ids() {
        let Some(meta) = level.client_meta(seat) else { continue };
        if meta.lifecycle != PlayerSessionStage::Alive {
            continue;
        }
        if let Some(pstate) = level.avatar(seat) {
            boxes.push(BodyBox::of_player(seat.0 as u16, pstate.origin));
        }
    }
    for remote in level.prediction_remote_bodies() {
        boxes.push(BodyBox::of_player(remote.client.0 as u16, remote.spot));
    }
    for (entnum, centre, solid) in level.packed_solid_movers() {
        boxes.push(BodyBox::of_packed_solid(entnum, centre, solid));
    }
    for spot in level.mantle_blockers() {
        boxes.push(BodyBox::of_landing_spot(spot.owner, spot.spot));
    }
    boxes
}

/// Whether two placed boxes overlap, edges touching not counting.
fn boxes_overlap(a: ([f32; 3], [f32; 3], [f32; 3]), b: ([f32; 3], [f32; 3], [f32; 3])) -> bool {
    let ((a_at, a_lo, a_hi), (b_at, b_lo, b_hi)) = (a, b);
    (0..3).all(|axis| a_at[axis] + a_lo[axis] < b_at[axis] + b_hi[axis] && a_at[axis] + a_hi[axis] > b_at[axis] + b_lo[axis])
}

/// Everything one move is clipped against besides the map itself.
#[derive(Clone, Copy)]
pub(super) struct SweepCollisionProvider<'a> {
    pub(super) brushes: &'a [SolidBrush],
    pub(super) bsp: &'a CollisionTreeData,
    pub(super) mesh: &'a CollisionTriangleData,
    /// Damage dealt to each glass piece (piece, damage); a broken piece stops nothing.
    pub(super) pane_harm: &'a [(u32, u16)],
    pub(super) player_boxes: &'a [BodyBox],
    /// The mover's own entity number: its own traces pass through its own boxes.
    pub(super) own_entity_slot: u16,
    pub(super) cmodels: &'a [asset_world::CollisionSubmodel],
    /// The brush models (doors, lifts), each with its entity number.
    pub(super) attached_solids: &'a [(u16, AttachedSolidShape)],
    /// Who owns each linked brush (same order as `attached_solids`).
    pub(super) linked_owners: &'a [crate::ServerModelSource],
    /// The traces that ended on a linked brush (fraction, end, its index); `None` where nobody
    /// reads which models a move touched (vehicles, benches).
    pub(super) struck: Option<&'a RefCell<Vec<(f32, [f32; 3], usize)>>>,
    /// The owners of the models the move touched (BO2's pmove keeps 32 at most).
    pub(super) touched: Option<&'a RefCell<Vec<crate::ServerModelSource>>>,
    /// Solid collision models as turned boxes (entity number, brush).
    pub(super) blockers: &'a [(u16, SolidBrush)],
    /// The script movers by entity number, for the platforms players ride
    /// ([`movement_t6::Bo2Collision::mover_pose`]).
    pub(super) movers: &'a [MoverSlot],
    /// The other scorestreak vehicles (only a vehicle's own move sees them).
    pub(super) vehicles: &'a [OtherVehicle],
    /// Black Ops II's own collision is on (a Black Ops II game always uses it).
    pub(super) gjk: bool,
    /// bo2zm: `forceslick` on him: every surface he touches is slick (SURF_SLICK 0x2).
    pub(super) slick: bool,
}

impl SweepCollisionProvider<'_> {
    pub(super) fn glass_stands(&self, piece: u16) -> bool {
        let wanted = u32::from(piece);
        let damage = self.pane_harm
            .iter()
            .filter(|entry| entry.0 == wanted)
            .map(|entry| entry.1)
            .next()
            .unwrap_or(0);
        crate::glass_and_props::shard_blocks_shots(damage)
    }
}

/// The script models each player's move touched this tick (BO2's pmove touch list); the map
/// scripts get their "touch" notifies from it.
#[derive(Resource, Default)]
pub(crate) struct PlayerModelTouches(pub(crate) Vec<(u32, crate::ServerModelSource)>);

/// bo2zm M3: the world's solid collision models (Nuketown's patch walls,
/// perk machines, the Mystery Box) as six-plane brushes.
pub(super) fn oriented_blocker_brushes(world: &StepWorldView) -> Vec<(u16, SolidBrush)> {
    world
        .oriented_blockers()
        .iter()
        .map(|b| {
            (
                b.ent_no,
                SolidBrush {
                    sides: b.planes().to_vec(),
                    solid_bits: crate::shot_trace::HARD_SURFACE_FLAG,
                    plane_material_flags: vec![b.face_bits; 6],
                    glass_hit_code: 0,
                    vertex_points: Vec::new(),
                    hit_surface_flags: 0,
                },
            )
        })
        .collect()
}

/// bo2zm M3: the move clipped against the turned boxes too. One he is
/// already inside never holds him (he walks out of it), as with entities'
/// boxes.
fn clip_move_to_blockers(so_far: Trace, blockers: &[(u16, SolidBrush)], input: SweepAsk) -> Trace {
    let SweepAsk { start, end, lo_corner: mins, hi_corner: maxs, collide_filter } = input;
    blockers.iter().fold(so_far, |nearest, (entnum, brush)| {
        let other = crate::sim_parts::t6_brushes_trace(&[brush], start, end, mins, maxs, collide_filter);
        let passes = other.starts_inside || other.fully_embedded || other.fraction >= nearest.fraction;
        if passes {
            nearest
        } else {
            Trace { impact_category: TraceHitType::Entity, struck_entity_num: *entnum, ..other }
        }
    })
}

/// A vehicle's move clipped against the other vehicles' turned boxes, as against the blockers
/// (a box it starts inside never holds it).
fn clip_move_to_vehicles(hit: Trace, vehicles: &[OtherVehicle], own_entity_slot: u16, input: SweepAsk) -> Trace {
    if input.collide_filter & VEHICLE_CONTENTS == 0 {
        return hit;
    }
    let others: Vec<(u16, SolidBrush)> = vehicles
        .iter()
        .filter(|other| other.entnum != own_entity_slot)
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
impl movement_t6::Bo2Collision for SweepCollisionProvider<'_> {
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
            if brush.solid_bits & mask == 0 {
                continue;
            }
            if let Some(shape_data) = brush_geom(brush, 1 << 32 | u64::from(id))
                && reaches(shape_data.lo_corner, shape_data.hi_corner)
            {
                out.push(shape_data);
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
                for root in first..first + usize::from(leaf.collision_box_total) {
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
                if node.sub_element_total != 0 {
                    stack.extend((u..u + usize::from(node.sub_element_total)).rev());
                    continue;
                }
                let Some(part) = map.partitions.get(u) else {
                    continue;
                };
                let Ok(first) = usize::try_from(part.leading_triangle) else {
                    continue;
                };
                let Some(triangle_list) = map.tri_indices.get(first..first + usize::from(part.tri_tally)) else {
                    continue;
                };
                // BO2 asks the tree's material.
                let material = map.materials.get(usize::from(node.material_index)).copied().unwrap_or_default();
                if material.pack_presence_bits & mask == 0 {
                    continue;
                }
                out.push(Geom {
                    shape: Shape::Partition { vertex_points: map.vertex_points, indices: triangle_list.as_flattened() },
                    axis: IDENTITY,
                    origin: [0.0; 3],
                    lo_corner: lo,
                    hi_corner: hi,
                    contents: material.pack_presence_bits,
                    surf_bits: asset_world::t5_surface_flags(material.surface_flags),
                    struck_entity_num: trace_t6::WORLD_ENTITY_SLOT,
                    sentient: false,
                    no_push_out: false,
                    key: 2 << 32 | u as u64,
                });
            }
        }

        // Brush models, turned and moved as their entity is.
        for (n, (entnum, linked)) in self.attached_solids.iter().enumerate() {
            let Some(collision_geometry) = self.cmodels.get(linked.collision_model_ref as usize) else {
                continue;
            };
            let reach = collision_geometry.radius + 1.0;
            if !reaches(linked.origin.map(|o| o - reach), linked.origin.map(|o| o + reach)) {
                continue;
            }
            let axis = game_math::euler_to_rows(linked.turn);
            let first = collision_geometry.initial_convex_solid as usize;
            let ids = self
                .bsp
                .brush_leaf_indices
                .get(first..first + usize::from(collision_geometry.solid_volume_total))
                .unwrap_or(&[]);
            let pieces = ids.iter().filter_map(|&id| self.brushes.get(usize::from(id)).map(|brush| (id, brush)));
            for (id, brush) in pieces.filter(|(_, brush)| brush.solid_bits & mask != 0) {
                let Some(mut shape_data) = brush_geom(brush, 3 << 32 | (n as u64) << 16 | u64::from(id)) else {
                    continue;
                };
                (shape_data.lo_corner, shape_data.hi_corner) = turned_box(shape_data.lo_corner, shape_data.hi_corner, &axis, linked.origin);
                shape_data.axis = axis;
                shape_data.origin = linked.origin;
                shape_data.struck_entity_num = *entnum;
                if reaches(shape_data.lo_corner, shape_data.hi_corner) {
                    out.push(shape_data);
                }
            }
        }

        // The other players' bodies (and the bo2zm zombies' boxes, and the
        // climbers' landing spots).
        {
            for (n, body) in self.player_boxes.iter().enumerate() {
                if body.owner == self.own_entity_slot || mask & body.contents == 0 {
                    continue;
                }
                let (body_mins, body_maxs) = body.bounds.unwrap_or((BODY_BOX_LOWER, BODY_BOX_UPPER));
                let hull = player_hull(body_mins, body_maxs, 0.0);
                let (lo, hi) = hull.calc_aabb();
                let at = body.origin;
                let shape_data = Geom {
                    key: 4 << 32 | n as u64,
                    struck_entity_num: body.entnum,
                    contents: body.contents,
                    sentient: body.contents == crate::sim_parts::SOLID_PLAYER_BODY,
                    no_push_out: true, surf_bits: 0,
                    shape: Shape::Player { hull },
                    origin: at,
                    axis: IDENTITY,
                    lo_corner: [at[0] + lo[0], at[1] + lo[1], at[2] + lo[2]],
                    hi_corner: [at[0] + hi[0], at[1] + hi[1], at[2] + hi[2]],
                };
                if reaches(shape_data.lo_corner, shape_data.hi_corner) {
                    out.push(shape_data);
                }
            }
        }

        // bo2mp: the other vehicles, each its model's collision piece turned
        // to its angles at its origin (never the vehicle moving).
        if mask & VEHICLE_CONTENTS != 0 {
            for (n, v) in self.vehicles.iter().enumerate() {
                let (lo, hi) = v.world;
                if v.entnum == self.own_entity_slot || !reaches(lo, hi) {
                    continue;
                }
                let shape = Shape::Brush { vertex_points: &v.vertex_points, lo_corner: v.mins, hi_corner: v.maxs, sides: &[] };
                out.push(Geom {
                    key: 5 << 32 | n as u64,
                    struck_entity_num: v.entnum,
                    contents: VEHICLE_CONTENTS,
                    sentient: false,
                    no_push_out: v.no_push_out,
                    surf_bits: 0,
                    shape,
                    origin: v.spot,
                    axis: v.frame,
                    lo_corner: lo,
                    hi_corner: hi,
                });
            }
        }
    }

    /// CM_GetWaterHeight: a point traced down through the feet, from 200
    /// above them to 200 below, that only water stops.
    fn water_height(&self, spot: [f32; 3]) -> f32 {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER, WATER_SEARCH};
        let start = [spot[0], spot[1], spot[2] + WATER_SEARCH];
        let end = [spot[0], spot[1], spot[2] - WATER_SEARCH];
        let point = [0.0; 3];
        let water = sweep_box_trace(self.brushes, self.bsp, self.mesh, start, end, point, point, CONTENTS_WATER, &|_| true);
        match water.fraction {
            f if f < 1.0 => start[2] + (end[2] - start[2]) * f,
            _ => NO_WATER,
        }
    }

    /// A script mover's pose on its path at `time`: where it was at the
    /// last command and is at this one (BG_EvaluateTrajectory on its
    /// position and its angles).
    fn mover_pose(&self, slot_no: i32, time: i32) -> Option<([f32; 3], [f32; 3])> {
        let i = self
            .movers
            .binary_search_by_key(&slot_no, |mover| mover.state.slot_index)
            .ok()?;
        let state = &self.movers[i].state;
        Some((
            entity_t6::evaluate_trajectory(&move_path_of(state), time),
            entity_t6::evaluate_trajectory(&turn_path_of(state), time),
        ))
    }
}

/// bo2mp: one brush as Black Ops II's collision sees it, in its own frame: a
/// box when it has only its six axial sides, else its corner points and its
/// other sides. None: a brush with sides but no corner points (not a Black
/// Ops II map's).
fn brush_geom(solid: &SolidBrush, key: u64) -> Option<movement_t6::gjk::Geom<'_>> {
    use movement_t6::gjk::{Geom, IDENTITY, Shape};
    let p = solid.sides.get(..6)?;
    let mins = [-p[1][3], -p[3][3], -p[5][3]];
    let maxs = [p[0][3], p[2][3], p[4][3]];
    let shape = if solid.sides.len() == 6 {
        Shape::Box {
            center: [0, 1, 2].map(|a| (mins[a] + maxs[a]) * 0.5),
            half: [0, 1, 2].map(|a| (maxs[a] - mins[a]) * 0.5),
            brush: true,
        }
    } else if solid.vertex_points.is_empty() {
        return None;
    } else {
        Shape::Brush { vertex_points: &solid.vertex_points, lo_corner: mins, hi_corner: maxs, sides: &solid.sides[6..] }
    };
    Some(Geom {
        key,
        struck_entity_num: trace_t6::WORLD_ENTITY_SLOT,
        contents: solid.solid_bits, sentient: false,
        no_push_out: false,
        surf_bits: solid.hit_surface_flags,
        shape,
        origin: [0.0; 3],
        axis: IDENTITY,
        lo_corner: mins,
        hi_corner: maxs,
    })
}

/// bo2mp: the world box around a local box turned by `axis` (rows) and moved
/// to `origin`.
fn turned_box(
    mins: [f32; 3],
    maxs: [f32; 3],
    axis: &[[f32; 3]; 3],
    spot: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = (spot, spot);
    for (row, (&low, &high)) in axis.iter().zip(mins.iter().zip(&maxs)) {
        for a in 0..3 {
            let (x, y) = (row[a] * low, row[a] * high);
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
    brushes: &'a [SolidBrush],
    bsp: &'a CollisionTreeData,
    mesh: &'a CollisionTriangleData,
    gjk: bool,
) -> impl movement_t6::Bo2Collision + 'a {
    SweepCollisionProvider {
        brushes,
        bsp,
        mesh,
        pane_harm: &[],
        player_boxes: &[],
        own_entity_slot: 0,
        cmodels: &[],
        attached_solids: &[],
        linked_owners: &[],
        struck: None,
        touched: None,
        blockers: &[],
        movers: &[],
        vehicles: &[],
        gjk,
        slick: false,
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
    spot: [f32; 3],
    frame: [[f32; 3]; 3],
    /// Its collision piece's corner points and their box, in its own frame.
    vertex_points: Vec<[f32; 3]>,
    mins: [f32; 3],
    maxs: [f32; 3],
    /// That box turned and placed: its world box, and as a brush (with the
    /// world box's sides) for the box traces.
    world: ([f32; 3], [f32; 3]),
    brush: SolidBrush,
    /// It is driven from afar (its def's `remoteControl`): BO2's move is
    /// never pushed out of one, as out of a player.
    no_push_out: bool,
}

impl OtherVehicle {
    /// None: no collision piece's corner points.
    pub(crate) fn new(entnum: u16, centre: [f32; 3], aim: [f32; 3], vertex_points: Vec<[f32; 3]>, no_push_out: bool) -> Option<Self> {
        let first = *vertex_points.first()?;
        let (mins, maxs) = vertex_points.iter().fold((first, first), |(lo, hi), p| {
            ([0, 1, 2].map(|a| lo[a].min(p[a])), [0, 1, 2].map(|a| hi[a].max(p[a])))
        });
        let axis = game_math::euler_to_rows(aim);
        let (lo, hi) = turned_box(mins, maxs, &axis, centre);
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
            let d = n[0] * centre[0] + n[1] * centre[1] + n[2] * centre[2];
            planes.push([n[0], n[1], n[2], d + maxs[row]]);
            planes.push([-n[0], -n[1], -n[2], -(d + mins[row])]);
        }
        let brush = SolidBrush {
            plane_material_flags: vec![0; planes.len()],
            sides: planes,
            solid_bits: VEHICLE_CONTENTS,
            glass_hit_code: 0,
            vertex_points: Vec::new(),
            hit_surface_flags: 0,
        };
        Some(Self {
            entnum,
            spot: centre,
            frame: axis,
            vertex_points,
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
    pub(super) attached_solids: Vec<(u16, AttachedSolidShape)>,
    pub(super) blockers: Vec<(u16, SolidBrush)>,
    pub(super) movers: Vec<MoverSlot>,
    pub(super) pane_harm: Vec<(u32, u16)>,
    /// The vehicle's own entity number: its own box never stops it.
    pub(super) own_entity_slot: u16,
    pub(super) vehicles: Vec<OtherVehicle>,
}

impl VehicleClip {
    pub(crate) fn gather(world: &mut StepWorldView, own_entity_slot: u16) -> Self {
        let movers = crate::step_view::gather_movers(world.ecs());
        let mover_numbers: std::collections::BTreeMap<_, u16> = movers
            .iter()
            .map(|mover| (mover.id, mover.state.slot_index as u16))
            .collect();
        let attached_solids = world
            .entity_hit_support()
            .iter()
            .flat_map(|support| {
                let carrier = support.owner.scripted_model().and_then(|id| mover_numbers.get(&id).copied());
                let entnum = carrier.unwrap_or(trace_t6::WORLD_ENTITY_SLOT);
                support.solid_brushes().iter().map(move |b| (entnum, b.clone()))
            })
            .filter(|(entnum, _)| *entnum != own_entity_slot)
            .collect();
        let mut blockers = oriented_blocker_brushes(world);
        blockers.retain(|(entnum, _)| *entnum != own_entity_slot);
        Self {
            bodies: alive_body_clips(world),
            attached_solids,
            blockers,
            movers,
            pane_harm: world.world_objects().pane_harm_entries(),
            own_entity_slot,
            vehicles: Vec::new(),
        }
    }

    /// The other scorestreak vehicles too (its own is left out).
    pub(crate) fn with_vehicles(mut self, mut vehicles: Vec<OtherVehicle>) -> Self {
        let own = self.own_entity_slot;
        vehicles.retain(|v| v.entnum != own);
        self.vehicles = vehicles;
        self
    }

    /// The map in `content` with what was gathered; `gjk` picks Black Ops
    /// II's own collision or the box traces.
    pub fn collision<'a>(&'a self, content: &'a crate::sim_parts::SimContent, gjk: bool) -> impl movement_t6::Bo2Collision + 'a {
        self.backend(
            content.collision_solids(),
            content.collision_tree(),
            content.collision_triangles(),
            &content.collision_submodel_set().models,
            gjk,
        )
    }

    pub(super) fn backend<'a>(
        &'a self,
        brushes: &'a [SolidBrush],
        bsp: &'a CollisionTreeData,
        mesh: &'a CollisionTriangleData,
        cmodels: &'a [asset_world::CollisionSubmodel],
        gjk: bool,
    ) -> SweepCollisionProvider<'a> {
        SweepCollisionProvider {
            brushes,
            bsp,
            mesh,
            pane_harm: &self.pane_harm,
            player_boxes: &self.bodies,
            own_entity_slot: self.own_entity_slot,
            cmodels,
            attached_solids: &self.attached_solids,
            // A vehicle's move reports no touches.
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &self.blockers,
            movers: &self.movers,
            vehicles: &self.vehicles,
            gjk,
            slick: false,
        }
    }
}

/// The lower part of his box (prone height) at `origin`, moved the shortest way out of any
/// brush: up to 4 units sideways or 18 up. Unchanged when it is already clear, or when no
/// spot that close is. The movement code's own fix tries only 1 unit, and Ascension's lander
/// spawn spots overlap its centre pole by 1-2 units.
pub(super) fn unstick_feet(backend: &SweepCollisionProvider<'_>, spot: [f32; 3]) -> [f32; 3] {
    let solid = |d: [f32; 3]| {
        let at = [spot[0] + d[0], spot[1] + d[1], spot[2] + d[2]];
        let tr = backend.trace(SweepAsk {
            start: at,
            end: at,
            lo_corner: super::STANDING_FOOTPRINT_LOW,
            hi_corner: [super::BODY_HALF_WIDTH, super::BODY_HALF_WIDTH, movement_t6::bridge::BELLY_HULL_TOP],
            collide_filter: crate::shot_trace::PLAYER_BODY_MASK,
        });
        tr.starts_inside || tr.fully_embedded
    };
    if !solid([0.0; 3]) { return spot; }
    let mut tries: Vec<[f32; 3]> = (-4..=4i8)
        .flat_map(|x| (-4..=4i8).map(move |y| [f32::from(x), f32::from(y), 0.0]))
        .chain((1..=18u8).map(|z| [0.0, 0.0, f32::from(z)]))
        .filter(|d| *d != [0.0; 3])
        .collect();
    tries.sort_by(|a, b| {
        let len = |d: &[f32; 3]| d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        len(a).total_cmp(&len(b))
    });
    tries
        .into_iter()
        .find(|&d| !solid(d))
        .map_or(spot, |d| [spot[0] + d[0], spot[1] + d[1], spot[2] + d[2]])
}

/// bo2zm test aid (BO2MP_T6_UNSTICK_LOG or BO2MP_T6_RIDELOG set): when `unstick_feet` moved him,
/// what his feet were inside: the map's own brushes, or which linked brush
/// model (its clip handle, origin and angles). At most 80 lines.
pub(super) fn unstick_log(backend: &SweepCollisionProvider<'_>, from: [f32; 3], till: [f32; 3]) {
    static UNSTICK_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if !*UNSTICK_ON.get_or_init(|| std::env::var_os("BO2MP_T6_UNSTICK_LOG").is_some() || std::env::var_os("BO2MP_T6_RIDELOG").is_some())
        || LINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 80
    {
        return;
    }
    let mins = super::STANDING_FOOTPRINT_LOW;
    let maxs = [super::BODY_HALF_WIDTH, super::BODY_HALF_WIDTH, movement_t6::bridge::BELLY_HULL_TOP];
    let mask = crate::shot_trace::PLAYER_BODY_MASK;
    let world_only = SweepCollisionProvider {
        player_boxes: &[],
        attached_solids: &[],
        linked_owners: &[],
        blockers: &[],
        vehicles: &[],
        ..*backend
    };
    let tr = world_only.trace(SweepAsk {
        start: from,
        end: from,
        lo_corner: mins,
        hi_corner: maxs,
        collide_filter: mask,
    });
    let map = tr.starts_inside || tr.fully_embedded;
    let models: Vec<String> = backend
        .attached_solids
        .iter()
        .filter(|(entnum, b)| {
            // The model alone, against nothing: does it hold his feet?
            let (t, _) = clip_move_to_bmodels_struck(
                Trace::new(),
                backend.bsp.t6,
                &backend.bsp.t6_model_brushes,
                std::iter::once((*entnum, b)),
                SweepAsk {
                    start: from,
                    end: from,
                    lo_corner: mins,
                    hi_corner: maxs,
                    collide_filter: mask,
                },
            );
            t.starts_inside || t.fully_embedded
        })
        .map(|(_, b)| {
            format!(
                " model {} at ({:.1} {:.1} {:.1}) angles ({:.1} {:.1} {:.1})",
                b.collision_model_ref, b.origin[0], b.origin[1], b.origin[2], b.turn[0], b.turn[1], b.turn[2]
            )
        })
        .collect();
    diag::info!(
        Sim,
        "bo2zm t6 unstick: ({:.2} {:.2} {:.2}) -> ({:.2} {:.2} {:.2}), inside map brushes {map} (contents {:#x}), linked models:{}",
        from[0],
        from[1],
        from[2],
        till[0],
        till[1],
        till[2],
        tr.cflags,
        if models.is_empty() { " none".to_string() } else { models.concat() }
    );
}

/// bo2zm down test (BO2MP_T6_DOWN set): while he is crouched or prone and not
/// in last stand, what fills the stand and crouch boxes at his feet, layer
/// by layer (after a revive the stand-up check found no room, 10-09). At
/// most 40 lines.
pub(super) fn stance_room_trace(backend: &SweepCollisionProvider<'_>, pstate: &AvatarState) {
    static DOWN_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static LINES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if !*DOWN_ON.get_or_init(|| std::env::var_os("BO2MP_T6_DOWN").is_some())
        || pstate.movement_mode != 0
        || pstate.movement_state_bits & 3 == 0
        || LINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 40
    {
        return;
    }
    let world_only = SweepCollisionProvider {
        player_boxes: &[],
        cmodels: &[],
        attached_solids: &[],
        linked_owners: &[],
        blockers: &[],
        vehicles: &[],
        ..*backend
    };
    let no_bodies = SweepCollisionProvider {
        player_boxes: &[],
        ..*backend
    };
    let show = |t: Trace| format!("{}{} {:?}:{}", t.fully_embedded, t.starts_inside, t.impact_category, t.struck_entity_num);
    let parts: Vec<String> = [
        ("stand", movement_t6::bridge::UPRIGHT_HULL_TOP),
        ("crouch", movement_t6::bridge::SQUAT_HULL_TOP),
    ]
    .into_iter()
    .map(|(called, top)| {
        let input = SweepAsk {
            start: pstate.origin, end: pstate.origin,
            lo_corner: super::STANDING_FOOTPRINT_LOW,
            hi_corner: [super::BODY_HALF_WIDTH, super::BODY_HALF_WIDTH, top],
            collide_filter: 0x0281_0011,
        };
        format!(
            "{called} world {} +models {} +bodies {}",
            show(world_only.trace(input)),
            show(no_bodies.trace(input)),
            show(backend.trace(input))
        )
    })
    .collect();
    // Which world part fills the stand hull (brushes or the triangle
    // mesh), and from how high up it is clear.
    let no_mesh = crate::sim_parts::CollisionTriangleData::default();
    let brushes_only = SweepCollisionProvider {
        mesh: &no_mesh,
        ..world_only
    };
    let mesh_only = SweepCollisionProvider {
        brushes: &[],
        ..world_only
    };
    let stand_at = |b: &SweepCollisionProvider<'_>, up: f32| {
        let o = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + up];
        b.trace(SweepAsk {
            start: o,
            end: o,
            lo_corner: super::STANDING_FOOTPRINT_LOW,
            hi_corner: super::STANDING_FOOTPRINT_HIGH,
            collide_filter: 0x0281_0011,
        })
    };
    let lift: Vec<String> = [0.25_f32, 1.0, 4.0, 16.0]
        .into_iter()
        .map(|up| format!("+{up} {}", show(stand_at(&world_only, up))))
        .collect();
    let m = stand_at(&mesh_only, 0.0);
    diag::info!(
        Sim,
        "bo2zm t6 stance room at {:?} pm_flags {:#x} target {}: {} | stand brushes {} mesh {} (contents {:#x} surf {:#x}) | world lifted {}",
        pstate.origin,
        pstate.movement_state_bits,
        pstate.eye_height_goal,
        parts.join(" | "),
        show(stand_at(&brushes_only, 0.0)),
        show(m),
        m.cflags,
        m.sflags,
        lift.join(" ")
    );
}

impl SolidProbe for SweepCollisionProvider<'_> {
    fn trace(&self, input: SweepAsk) -> trace_t6::Trace {
        let SweepAsk { start, end, lo_corner: mins, hi_corner: maxs, collide_filter } = input;
        let glass = |piece| self.glass_stands(piece);
        let map_hit = sweep_box_trace(self.brushes, self.bsp, self.mesh, start, end, mins, maxs, collide_filter, &glass);
        let (after_models, struck_model) = clip_move_to_bmodels_struck(
            map_hit,
            self.bsp.t6,
            &self.bsp.t6_model_brushes,
            self.attached_solids.iter().map(|(entnum, brush)| (*entnum, brush)),
            input,
        );

        let mut hit = sweep_against_players(after_models, self.bsp.t6, self.player_boxes, self.own_entity_slot, input);
        hit = clip_move_to_blockers(hit, self.blockers, input);
        hit = clip_move_to_vehicles(hit, self.vehicles, self.own_entity_slot, input);
        if self.slick {
            hit.sflags |= 2;
        }

        // Note the brush model that stopped the move, when nothing nearer replaced it.
        let model_decided = hit.fraction == after_models.fraction
            && hit.impact_category == after_models.impact_category
            && hit.struck_entity_num == after_models.struck_entity_num;
        if let (Some(index), Some(log), true) = (struck_model, self.struck, model_decided) {
            log.borrow_mut()
                .push((hit.fraction, hit.finish_location(input.start, input.end), index));
        }
        hit
    }

    fn brush_against(&self, trace: &trace_t6::Trace, finish_location: [f32; 3]) {
        let (Some(log), Some(touched)) = (self.struck, self.touched) else {
            return;
        };
        let index = log
            .borrow()
            .iter()
            .rev()
            .find(|(fraction, end, _)| *fraction == trace.fraction && *end == finish_location)
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
fn stop_deeper_in_box(hit: &mut Trace, body: &BodyBox, input: &SweepAsk) -> bool {
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
    hit.impact_category = TraceHitType::Entity;
    hit.struck_entity_num = body.entnum;
    hit.walkable = false;
    true
}

/// The move clipped against the other players' bodies. On a Black Ops II map each body is its
/// temporary box (`CM_TempBoxModel`), traced as a capsule.
fn sweep_against_players(
    so_far: Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    bodies: &[BodyBox],
    own_entity_slot: u16,
    given: SweepAsk,
) -> Trace {
    // A move stopped where it began meets nobody.
    let mut hit = so_far;
    let stopped_at_start = hit.fraction == 0.0;
    // bo2zm: players and zombies only stop traces that ask for bodies: BO2's
    // stand-up (PM_CanStand), lie-down (0x810011) and mantle checks leave them out.
    if stopped_at_start || given.collide_filter & crate::sim_parts::SOLID_PLAYER_BODY == 0 {
        return hit;
    }
    for corpse in bodies.iter().filter(|corpse| corpse.owner != own_entity_slot) {
        let (body_mins, body_maxs) = corpse.extent();
        let inside_packed_box = corpse.bounds.is_some()
            && boxes_overlap(
                (given.start, given.lo_corner, given.hi_corner),
                (corpse.origin, body_mins, body_maxs),
            );
        if inside_packed_box {
            if stop_deeper_in_box(&mut hit, corpse, &given) {
                return hit;
            }
            continue;
        }

        // With no map (no brushes at all) the temporary box needs none.
        let empty_map = clipmap_t6::ClipMap::default();
        let map = t6.unwrap_or(&empty_map);
        let temp_box = crate::sim_parts::T6Model::TempBox { mins: body_mins, maxs: body_maxs, contents: corpse.contents };
        let SweepAsk { start, end, lo_corner: mins, hi_corner: maxs, collide_filter } = given;
        let other =
            crate::sim_parts::t6_transformed_trace(map, start, end, mins, maxs, temp_box, collide_filter, corpse.origin, [0.0; 3]);
        let nearer = other.fraction < hit.fraction;
        if !nearer && !other.starts_inside && !other.fully_embedded {
            continue;
        }
        let started_solid = hit.starts_inside | other.starts_inside;
        let all_solid = hit.fully_embedded | other.fully_embedded;
        if nearer {
            hit = Trace { impact_category: TraceHitType::Entity, struck_entity_num: corpse.entnum, ..other };
        }
        hit.starts_inside = started_solid;
        hit.fully_embedded = all_solid;
    }
    hit
}
