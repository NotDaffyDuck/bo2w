use super::*;
use trace_t6::{TraceExtents, trace_through_brush};

// A small map: the plane x = 0 splits leaf 0 (x >= 0), which holds a box
// brush, from leaf 1 (x < 0), which holds a two-triangle floor at z = 0.
const BRUSH_LIST: [u16; 1] = [0];
const VERTS: [[f32; 3]; 4] = [
    [-128.0, -64.0, 0.0],
    [0.0, -64.0, 0.0],
    [0.0, 64.0, 0.0],
    [-128.0, 64.0, 0.0],
];
// Wound so the face looks up.
const TRIS: [[u16; 3]; 2] = [[0, 2, 1], [0, 3, 2]];
const WALKABLE: [u8; 1] = [0xff];

fn brush_leaf() -> Leaf {
    Leaf {
        brush_contents: 1,
        mins: [0.0, -1000.0, -1000.0],
        maxs: [1000.0, 1000.0, 1000.0],
        leaf_brush_node: 1,
        ..Leaf::default()
    }
}

fn with_map<R>(f: impl FnOnce(&ClipMap<'_>, &mut CheckCount<'_>) -> R) -> R {
    let brushes = [Brush {
        mins: [16.0, -16.0, 0.0],
        maxs: [48.0, 16.0, 32.0],
        contents: 1,
        ..Brush::default()
    }];
    let nodes = [Node {
        plane: CPlane::new([1.0, 0.0, 0.0], 0.0),
        children: [-1, -2],
    }];
    let leafs = [
        brush_leaf(),
        Leaf {
            coll_aabb_count: 1,
            terrain_contents: 1,
            mins: [-1000.0, -1000.0, -1000.0],
            maxs: [0.0, 1000.0, 1000.0],
            ..Leaf::default()
        },
    ];
    let leaf_brush_nodes = [
        LeafBrushNode::default(),
        LeafBrushNode {
            leaf_brush_count: 1,
            contents: 1,
            brushes: &BRUSH_LIST,
            ..LeafBrushNode::default()
        },
    ];
    let materials = [ClipMaterial {
        surface_flags: 0x123,
        content_flags: 1,
    }];
    let aabb_trees = [AabbTree {
        origin: [-64.0, 0.0, 0.0],
        half_size: [64.0, 64.0, 0.0],
        ..AabbTree::default()
    }];
    let partitions = [Partition {
        tri_count: 2,
        first_tri: 0,
    }];
    let cmodels = [
        CModel::default(),
        CModel {
            leaf: brush_leaf(),
            ..CModel::default()
        },
    ];
    let map = ClipMap {
        nodes: &nodes,
        leafs: &leafs,
        leaf_brush_nodes: &leaf_brush_nodes,
        brushes: &brushes,
        materials: &materials,
        aabb_trees: &aabb_trees,
        partitions: &partitions,
        verts: &VERTS,
        tri_indices: &TRIS,
        tri_edge_is_walkable: &WALKABLE,
        cmodels: &cmodels,
        temp: TempModels::default(),
    };
    let mut marks = [0u16; 1];
    let mut check = CheckCount::new(&mut marks);
    f(&map, &mut check)
}

const POINT: [f32; 3] = [0.0; 3];

#[test]
fn a_point_dropped_on_the_floor_stops_on_it() {
    let t = with_map(|map, check| map.box_trace(check, [-40.0, -30.0, 64.0], [-40.0, -30.0, -64.0], POINT, POINT, 0, 1));
    assert_eq!(t.fraction, 0.5);
    assert_eq!(t.normal, [0.0, 0.0, 1.0]);
    assert!(t.walkable && !t.startsolid);
    assert_eq!((t.sflags, t.cflags, t.hit_partition), (0x123, 1, 0));
    assert_eq!(t.bone_index, 0xfe);
}

#[test]
fn a_player_capsule_lands_on_the_floor() {
    let (mins, maxs) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let t = with_map(|map, check| map.box_trace(check, [-64.0, 0.0, 100.0], [-64.0, 0.0, -100.0], mins, maxs, 0, 1));
    assert!((t.fraction - 99.875 / 200.0).abs() < 1e-5, "{}", t.fraction);
    assert_eq!(t.normal, [0.0, 0.0, 1.0]);
    assert!(t.walkable && !t.startsolid);
}

#[test]
fn a_capsule_brushing_past_a_floor_corner_is_stopped_by_it() {
    // The lower sphere drops past the floor's outer corner at (-128, -64).
    let (mins, maxs) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let t = with_map(|map, check| map.box_trace(check, [-138.0, -74.0, 50.0], [-138.0, -74.0, -50.0], mins, maxs, 0, 1));
    // 14.14 to the side of the corner, the 15 sphere touches it with its
    // centre 5 up: 0.6 of the way down, less the clip epsilon.
    assert!(t.fraction > 0.59 && t.fraction < 0.6, "{}", t.fraction);
    assert!(t.normal[0] < 0.0 && t.normal[1] < 0.0 && t.normal[2] > 0.0, "{:?}", t.normal);
    let len = dot(t.normal, t.normal);
    assert!((len - 1.0).abs() < 1e-3, "{len}");
}

#[test]
fn a_point_trace_stops_at_the_brush() {
    let t = with_map(|map, check| map.box_trace(check, [100.0, 0.0, 16.0], [0.0, 0.0, 16.0], POINT, POINT, 0, 1));
    let mut want = Trace::new();
    let tw = TraceWork::new([100.0, 0.0, 16.0], [0.0, 0.0, 16.0], POINT, POINT, 1);
    with_map(|map, _| trace_through_brush(&tw, &map.brushes[0], &mut want));
    assert_eq!(t.fraction, want.fraction);
    assert!(t.fraction > 0.5 && t.fraction < 0.52, "{}", t.fraction);
    assert_eq!(t.normal, [1.0, 0.0, 0.0]);
    // The same through the brush model.
    let m = with_map(|map, check| map.box_trace(check, [100.0, 0.0, 16.0], [0.0, 0.0, 16.0], POINT, POINT, 1, 1));
    assert_eq!(m.fraction, t.fraction);
}

#[test]
fn the_temporary_models_trace_as_nothing_for_now() {
    for model in [CAPSULE_MODEL_HANDLE, TEMP_BRUSH_MODEL_HANDLE, 7] {
        let t = with_map(|map, check| map.box_trace(check, [100.0, 0.0, 16.0], [0.0, 0.0, 16.0], POINT, POINT, model, 1));
        assert_eq!(t.fraction, 1.0, "model {model:#x}");
    }
}

#[test]
fn a_box_inside_the_brush_is_all_solid() {
    let b = [1.0; 3];
    let nb = [-1.0; 3];
    let t = with_map(|map, check| map.box_trace(check, [32.0, 0.0, 16.0], [32.0, 0.0, 16.0], nb, b, 0, 1));
    assert!(t.allsolid && t.startsolid);
    let t = with_map(|map, check| map.box_trace(check, [200.0, 0.0, 16.0], [200.0, 0.0, 16.0], nb, b, 0, 1));
    assert!(!t.allsolid && !t.startsolid);
    let t = with_map(|map, check| map.box_trace(check, [32.0, 0.0, 16.0], [32.0, 0.0, 16.0], nb, b, 1, 1));
    assert!(t.allsolid);
}

#[test]
fn a_capsule_sunk_into_the_floor_is_all_solid() {
    let (mins, maxs) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let at = [-64.0, 0.0, -10.0];
    let t = with_map(|map, check| map.box_trace(check, at, at, mins, maxs, 0, 1));
    assert!(t.allsolid && t.startsolid);
    assert_eq!(t.sflags, 0x123);
    let at = [-64.0, 0.0, 10.0];
    let t = with_map(|map, check| map.box_trace(check, at, at, mins, maxs, 0, 1));
    assert!(!t.allsolid);
    // A sphere (no height) sunk into it.
    let at = [-64.0, 0.0, 5.0];
    let t = with_map(|map, check| map.box_trace(check, at, at, [-8.0; 3], [8.0; 3], 0, 1));
    assert!(t.allsolid);
}

#[test]
fn a_partition_already_looked_at_this_trace_is_skipped() {
    let t = with_map(|map, check| {
        check.global = 4;
        check.partitions[0] = 5;
        map.box_trace(check, [-40.0, -30.0, 64.0], [-40.0, -30.0, -64.0], POINT, POINT, 0, 1)
    });
    assert_eq!(t.fraction, 1.0);
    assert_eq!(t.sflags, 0);
}

#[test]
fn point_contents_finds_the_brush() {
    with_map(|map, _| {
        assert_eq!(point_contents(map, [32.0, 0.0, 16.0], 0), 1);
        assert_eq!(point_contents(map, [200.0, 0.0, 16.0], 0), 0);
        assert_eq!(point_contents(map, [-64.0, 0.0, 0.0], 0), 0);
        assert_eq!(point_contents(map, [32.0, 0.0, 16.0], 1), 1);
        assert_eq!(point_contents(map, [32.0, 0.0, 16.0], 9), 0);
        assert_eq!(transformed_point_contents(map, [132.0, 0.0, 16.0], 1, [100.0, 0.0, 0.0], [0.0; 3]), 1);
        assert_eq!(transformed_point_contents(map, [0.0, 32.0, 16.0], 1, [0.0; 3], [0.0, 90.0, 0.0]), 1);
        assert_eq!(transformed_point_contents(map, [32.0, 0.0, 16.0], 1, [0.0; 3], [0.0, 90.0, 0.0]), 0);
    });
}

#[test]
fn leaf_lookups() {
    with_map(|map, _| {
        assert_eq!(point_leafnum(map, [5.0, 0.0, 0.0]), 0);
        assert_eq!(point_leafnum(map, [-5.0, 0.0, 0.0]), 1);
        assert_eq!(point_leafnum(map, [0.0, 0.0, 0.0]), 0);
        let mut buf = [0u16; 4];
        let mut ll = LeafList::new(&mut buf, [-1.0; 3], [1.0; 3]);
        box_leafnums(map, &mut ll, 0);
        assert_eq!(ll.leafs(), &[0, 1]);
        assert_eq!((ll.last_leaf, ll.overflowed), (1, false));
        let mut ll = LeafList::new(&mut buf, [-9.0; 3], [-1.0; 3]);
        box_leafnums(map, &mut ll, 0);
        assert_eq!(ll.leafs(), &[1]);
        let mut one = [0u16; 1];
        let mut ll = LeafList::new(&mut one, [-1.0; 3], [1.0; 3]);
        box_leafnums(map, &mut ll, 0);
        assert_eq!((ll.leafs(), ll.overflowed), (&[0u16][..], true));
    });
    let plane = CPlane::new([0.0, -1.0, 0.0], 2.0);
    assert_eq!((plane.kind, plane.signbits), (3, 2));
    assert_eq!(CPlane::new([0.0, 0.0, 1.0], 0.0).kind, 2);
    assert_eq!(box_on_plane_side([0.0, -9.0, 0.0], [1.0, -3.0, 1.0], &plane), 1);
    assert_eq!(box_on_plane_side([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], &plane), 2);
    assert_eq!(box_on_plane_side([0.0, -3.0, 0.0], [1.0, 0.0, 1.0], &plane), 3);
}

#[test]
fn closest_point_on_tri_regions() {
    let (b, e0, e1) = ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    for (p, want) in [
        ([0.25, 0.25, 5.0], [0.25, 0.25, 0.0]),
        ([-1.0, -1.0, 0.0], [0.0, 0.0, 0.0]),
        ([2.0, -1.0, 0.0], [1.0, 0.0, 0.0]),
        ([1.0, 1.0, 0.0], [0.5, 0.5, 0.0]),
        ([-1.0, 0.5, 0.0], [0.0, 0.5, 0.0]),
        ([0.5, -1.0, 0.0], [0.5, 0.0, 0.0]),
        ([-1.0, 3.0, 0.0], [0.0, 1.0, 0.0]),
        ([3.0, 3.0, 0.0], [0.5, 0.5, 0.0]),
    ] {
        assert_eq!(closest_point_on_tri(p, b, e0, e1), want, "{p:?}");
    }
}

#[test]
fn capsule_against_triangle() {
    let tri = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
    assert!(does_capsule_intersect_triangle([2.0, 2.0, 5.0], [2.0, 2.0, -5.0], 1.0, tri));
    assert!(!does_capsule_intersect_triangle([2.0, 2.0, 10.0], [2.0, 2.0, 3.0], 1.0, tri));
    assert!(does_capsule_intersect_triangle([2.0, 2.0, 10.0], [2.0, 2.0, 0.5], 1.0, tri));
    assert!(!does_capsule_intersect_triangle([20.0, 20.0, 5.0], [20.0, 20.0, -5.0], 1.0, tri));
    // Through the plane just past the long edge, but close enough to touch it.
    assert!(does_capsule_intersect_triangle([5.6, 5.6, 3.0], [5.6, 5.6, -3.0], 1.0, tri));
    assert!(!does_capsule_intersect_triangle([6.5, 6.5, 3.0], [6.5, 6.5, -3.0], 1.0, tri));
}

#[test]
fn segment_distances() {
    let d = segment_distance_sq([0.0; 3], [1.0, 0.0, 0.0], [0.5, -1.0, 1.0], [0.0, 2.0, 0.0]);
    assert_eq!(d, 1.0);
    assert_eq!(segment_distance_sq([0.0; 3], [1.0, 0.0, 0.0], [3.0, 0.0, 0.0], [1.0, 0.0, 0.0]), 4.0);
    assert_eq!(segment_distance_sq([0.0; 3], [0.0; 3], [0.0, 2.0, 0.0], [0.0; 3]), 4.0);
}

#[test]
fn extents_against_a_box() {
    let ext = TraceExtents::new([0.0; 3], [10.0, 0.0, 0.0]);
    let (mins, maxs) = ([4.0, -1.0, -1.0], [6.0, 1.0, 1.0]);
    assert!(intersect_extents_aabb(&ext, mins, maxs, 1.0));
    assert!(!intersect_extents_aabb(&ext, mins, maxs, 0.3));
    let off = TraceExtents::new([0.0, 5.0, 0.0], [10.0, 5.0, 0.0]);
    assert!(!intersect_extents_aabb(&off, mins, maxs, 1.0));
}

#[test]
fn angles_to_axis_turns() {
    assert_eq!(angles_to_axis([0.0; 3]), [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let a = angles_to_axis([0.0, 90.0, 0.0]);
    assert!((a[0][1] - 1.0).abs() < 1e-6 && (a[1][0] + 1.0).abs() < 1e-6);
}

#[test]
fn a_clear_sight_line_is_zero() {
    // Over the brush and the floor.
    let hit = with_map(|map, check| map.box_sight_trace(check, 0, [100.0, 0.0, 64.0], [-100.0, 0.0, 64.0], POINT, POINT, 0, 1));
    assert_eq!(hit, 0);
    let hit = with_map(|map, check| map.sight_trace_point(check, 0, [100.0, 0.0, 64.0], [-100.0, 0.0, 64.0], 1, &[]));
    assert_eq!(hit, 0);
}

#[test]
fn a_sight_line_through_the_brush_names_it() {
    let (start, end) = ([100.0, 0.0, 16.0], [0.0, 0.0, 16.0]);
    let (mins, maxs) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    for hint in [0, 1, 5] {
        let hit = with_map(|map, check| map.box_sight_trace(check, hint, start, end, POINT, POINT, 0, 1));
        assert_eq!(hit, 1, "hint {hint}");
        let hit = with_map(|map, check| map.sight_trace_point(check, hint, start, end, 1, &[]));
        assert_eq!(hit, 1, "hint {hint}");
    }
    let hit = with_map(|map, check| map.box_sight_trace(check, 0, start, end, mins, maxs, 0, 1));
    assert_eq!(hit, 1);
    // Other contents see through it, unless it is the hint: BO2 tries the
    // hinted brush without looking at its contents.
    let hit = with_map(|map, check| map.box_sight_trace(check, 0, start, end, POINT, POINT, 0, 2));
    assert_eq!(hit, 0);
    let hit = with_map(|map, check| map.box_sight_trace(check, 1, start, end, POINT, POINT, 0, 2));
    assert_eq!(hit, 1);
    // So does a line that passes beside it.
    let hit = with_map(|map, check| map.box_sight_trace(check, 1, [100.0, 40.0, 16.0], [0.0, 40.0, 16.0], POINT, POINT, 0, 1));
    assert_eq!(hit, 0);
    // The brush model holds the same brush.
    let hit = with_map(|map, check| map.box_sight_trace(check, 0, start, end, POINT, POINT, 1, 1));
    assert_eq!(hit, 1);
}

#[test]
fn a_sight_line_into_the_floor_names_the_terrain() {
    let (start, end) = ([-40.0, -30.0, 64.0], [-40.0, -30.0, -64.0]);
    let hit = with_map(|map, check| map.box_sight_trace(check, 0, start, end, POINT, POINT, 0, 1));
    assert_eq!(hit, 2);
    let hit = with_map(|map, check| map.sight_trace_point(check, 0, start, end, 1, &[]));
    assert_eq!(hit, 2);
}

#[test]
fn a_sight_point_with_a_list_looks_only_at_it() {
    let floor = ([-40.0, -30.0, 64.0], [-40.0, -30.0, -64.0]);
    let brush = ([100.0, 0.0, 16.0], [0.0, 0.0, 16.0]);
    let tree = [SightPrim::AabbTree(0)];
    let box_ = [SightPrim::Brush(0)];
    let hit = with_map(|map, check| map.sight_trace_point(check, 0, floor.0, floor.1, 1, &tree));
    assert_eq!(hit, 2);
    let hit = with_map(|map, check| map.sight_trace_point(check, 0, floor.0, floor.1, 1, &box_));
    assert_eq!(hit, 0);
    let hit = with_map(|map, check| map.sight_trace_point(check, 0, brush.0, brush.1, 1, &box_));
    assert_eq!(hit, 1);
    let hit = with_map(|map, check| map.sight_trace_point(check, 0, brush.0, brush.1, 1, &tree));
    assert_eq!(hit, 0);
}

#[test]
fn the_temporary_box_blocks_sight() {
    let (mins, maxs) = ([-15.0, -15.0, 0.0], [15.0, 15.0, 72.0]);
    let mut map = ClipMap::default();
    let handle = map.temp_box_model([-16.0, -16.0, 0.0], [16.0, 16.0, 72.0], 1);
    let mut check = CheckCount::new(&mut []);
    let mut sight = |start, end, contents| map.box_sight_trace(&mut check, 0, start, end, mins, maxs, handle, contents);
    assert_eq!(sight([100.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1), -1);
    assert_eq!(sight([100.0, 0.0, 0.0], [0.0, 0.0, 0.0], 2), 0);
    assert_eq!(sight([100.0, 0.0, 200.0], [-100.0, 0.0, 200.0], 1), 0);
    assert_eq!(sight([100.0, 100.0, 0.0], [-100.0, 100.0, 0.0], 1), 0);
    // Starting inside it, or dropping onto it from above.
    assert_eq!(sight([4.0, 0.0, 0.0], [4.0, 0.0, 0.0], 1), -1);
    assert_eq!(sight([0.0, 0.0, 200.0], [0.0, 0.0, 0.0], 1), -1);
}

#[test]
fn the_temporary_brushes_block_sight() {
    static BRUSHES: [Brush<'static>; 1] = [Brush {
        mins: [16.0, -16.0, 0.0],
        maxs: [48.0, 16.0, 32.0],
        contents: 1,
        sides: &[],
        axial_cflags: [[1; 3]; 2],
        axial_sflags: [[0; 3]; 2],
    }];
    let mut map = ClipMap::default();
    let handle = map.temp_brush_model(&BRUSHES);
    let mut check = CheckCount::new(&mut []);
    let hit = map.box_sight_trace(&mut check, 0, [100.0, 0.0, 16.0], [0.0, 0.0, 16.0], POINT, POINT, handle, 1);
    assert_eq!(hit, -1);
    let hit = map.box_sight_trace(&mut check, 0, [100.0, 0.0, 64.0], [0.0, 0.0, 64.0], POINT, POINT, handle, 1);
    assert_eq!(hit, 0);
}

// World sectors over a 8192 by 2048 world: the root splits x at 0.
struct Game {
    contents: [u32; 16],
    bounds: [([f32; 3], [f32; 3]); 16],
}

impl Game {
    fn new() -> Self {
        Self {
            contents: [1; 16],
            bounds: [([0.0; 3], [0.0; 3]); 16],
        }
    }
}

impl LinkedEntities for Game {
    fn contents(&self, num: usize) -> u32 {
        self.contents[num]
    }

    fn abs_bounds(&self, num: usize) -> ([f32; 3], [f32; 3]) {
        self.bounds[num]
    }
}

fn sector_models() -> [CModel; 2] {
    let leaf = Leaf {
        brush_contents: 1,
        ..Leaf::default()
    };
    [
        CModel {
            mins: [-4096.0, -1024.0, -512.0],
            maxs: [4096.0, 1024.0, 512.0],
            leaf,
            ..CModel::default()
        },
        CModel {
            leaf,
            ..CModel::default()
        },
    ]
}

fn with_world<R>(f: impl FnOnce(&ClipMap<'_>, &mut World) -> R) -> R {
    let cmodels = sector_models();
    let cm = ClipMap {
        cmodels: &cmodels,
        ..ClipMap::default()
    };
    let mut world = World::new();
    world.clear(&cm);
    f(&cm, &mut world)
}

/// Link entity `num` with model 1 over `absmin`..`absmax`.
fn link(cm: &ClipMap<'_>, world: &mut World, game: &mut Game, num: usize, absmin: [f32; 3], absmax: [f32; 3]) {
    game.bounds[num] = (absmin, absmax);
    world.link_entity(cm, &*game, &mut [], num, absmin, absmax, 1);
}

#[test]
fn clearing_the_world_splits_the_root_across_its_longer_side() {
    with_world(|_, world| {
        assert_eq!(world.free_head, 2);
        assert_eq!(world.sectors[1].axis, 0);
        assert_eq!(world.sectors[1].dist, 0.0);
        assert_eq!(world.sectors[2].parent, 3);
        assert_eq!(world.sectors[1022].parent, 1023);
        assert_eq!(world.sectors[1023].parent, 0);
    });
}

#[test]
fn an_entity_on_one_side_moves_into_a_new_child() {
    with_world(|cm, world| {
        let mut game = Game::new();
        link(cm, world, &mut game, 3, [100.0, -10.0, 0.0], [120.0, 10.0, 0.0]);
        // Past the split is child 0, made from the half the walk narrowed to.
        assert_eq!(world.sectors[1].children, [2, 0]);
        assert_eq!(world.sectors[2].parent, 1);
        assert_eq!(world.sectors[2].axis, 0);
        assert_eq!(world.sectors[2].dist, 2048.0);
        assert_eq!(world.entities[3].world_sector, 2);
        assert_eq!(world.sectors[2].entities, 4);
        assert_eq!(world.sectors[1].entities, 0);
        assert_eq!(world.free_head, 3);
        assert_eq!(world.sectors[1].contents_entities, 1);

        world.unlink_entity(&game, 3);
        assert_eq!(world.entities[3].world_sector, 0);
        assert_eq!(world.sectors[1].children, [0; 2]);
        assert_eq!(world.free_head, 2);
        assert_eq!(world.sectors[2].parent, 3);
        assert_eq!(world.sectors[1].contents_entities, 0);
    });
}

#[test]
fn entities_across_the_split_stay_in_entity_order() {
    with_world(|cm, world| {
        let mut game = Game::new();
        for num in [5, 2, 9] {
            link(cm, world, &mut game, num, [-10.0; 3], [10.0; 3]);
        }
        assert_eq!(world.sectors[1].entities, 3);
        assert_eq!(world.entities[2].next_entity_in_world_sector, 6);
        assert_eq!(world.entities[5].next_entity_in_world_sector, 10);
        assert_eq!(world.entities[9].next_entity_in_world_sector, 0);
        assert_eq!(world.sectors[1].children, [0; 2]);

        world.unlink_entity(&game, 5);
        assert_eq!(world.entities[2].next_entity_in_world_sector, 10);
    });
}

#[test]
fn relinking_in_the_same_sector_only_moves_the_box() {
    with_world(|cm, world| {
        let mut game = Game::new();
        link(cm, world, &mut game, 1, [-10.0; 3], [10.0; 3]);
        link(cm, world, &mut game, 1, [-20.0; 3], [5.0; 3]);
        assert_eq!(world.entities[1].world_sector, 1);
        assert_eq!(world.entities[1].link_min, [-20.0; 3]);
        assert_eq!(world.entities[1].link_max, [5.0; 3]);

        // Moved wholly across the split, it is unlinked and filed again.
        link(cm, world, &mut game, 1, [-300.0; 3], [-200.0; 3]);
        assert_eq!(world.entities[1].world_sector, 2);
        assert_eq!(world.sectors[1].children, [0, 2]);
        assert_eq!(world.sectors[2].dist, -2048.0);
    });
}

#[test]
fn a_model_with_no_contents_unlinks_the_entity() {
    with_world(|cm, world| {
        let mut game = Game::new();
        link(cm, world, &mut game, 4, [-10.0; 3], [10.0; 3]);
        world.link_entity(cm, &game, &mut [], 4, [-10.0; 3], [10.0; 3], 7);
        assert_eq!(world.entities[4].world_sector, 0);
        assert_eq!(world.sectors[1].entities, 0);
    });
}

#[test]
fn area_entities_finds_touching_boxes_with_the_mask() {
    with_world(|cm, world| {
        let mut game = Game::new();
        game.contents[6] = 2;
        link(cm, world, &mut game, 1, [-10.0; 3], [10.0; 3]);
        link(cm, world, &mut game, 4, [100.0, 0.0, 0.0], [120.0, 10.0, 10.0]);
        link(cm, world, &mut game, 6, [100.0, 0.0, 0.0], [120.0, 10.0, 10.0]);
        link(cm, world, &mut game, 8, [-500.0, 0.0, 0.0], [-400.0, 10.0, 10.0]);

        let mut list = [0; 8];
        let n = world.area_entities(&game, [0.0; 3], [200.0, 50.0, 50.0], 1, &mut list);
        assert_eq!(&list[..n], &[1, 4]);
        let n = world.area_entities(&game, [0.0; 3], [200.0, 50.0, 50.0], 2, &mut list);
        assert_eq!(&list[..n], &[6]);
        let n = world.area_entities(&game, [-600.0; 3], [600.0; 3], 3, &mut list);
        assert_eq!(n, 4);
        // A full list stops the walk.
        let n = world.area_entities(&game, [-600.0; 3], [600.0; 3], 3, &mut list[..2]);
        assert_eq!(n, 2);
    });
}

#[test]
fn static_models_are_filed_on_their_side() {
    let cmodels = sector_models();
    let cm = ClipMap {
        cmodels: &cmodels,
        ..ClipMap::default()
    };
    let mut models = [
        StaticModel {
            contents: 1,
            absmin: [-300.0; 3],
            absmax: [-200.0; 3],
            ..StaticModel::default()
        },
        StaticModel {
            contents: 4,
            absmin: [-10.0; 3],
            absmax: [10.0; 3],
            ..StaticModel::default()
        },
        StaticModel {
            absmin: [-300.0; 3],
            absmax: [-200.0; 3],
            ..StaticModel::default()
        },
    ];
    let mut world = World::new();
    world.link_world(&cm, &Game::new(), &mut models);
    assert_eq!(world.sectors[1].static_models, 2);
    assert_eq!(world.sectors[1].children, [0, 2]);
    assert_eq!(world.sectors[2].static_models, 1);
    assert_eq!(world.sectors[1].contents_static_models, 5);
    assert_eq!(world.sectors[2].contents_static_models, 1);
    assert_eq!(models[0].next_model_in_world_sector, 0);
}

// Static models. A floor triangle at local z = 0 over (0,0)-(10,0)-(0,10),
// and a wall triangle at local x = 0 facing -x over y and z 0 to 10.
const FLOOR_TRI: [XModelCollTri; 1] = [XModelCollTri {
    plane: [0.0, 0.0, 1.0, 0.0],
    svec: [0.1, 0.0, 0.0, 0.0],
    tvec: [0.0, 0.1, 0.0, 0.0],
}];
const WALL_TRI: [XModelCollTri; 1] = [XModelCollTri {
    plane: [-1.0, 0.0, 0.0, 0.0],
    svec: [0.0, 0.1, 0.0, 0.0],
    tvec: [0.0, 0.0, 0.1, 0.0],
}];

fn coll_surf(tris: &[XModelCollTri]) -> XModelCollSurf<'_> {
    XModelCollSurf {
        tris,
        mins: [-1.0, -1.0, -1.0],
        maxs: [10.0, 10.0, 10.0],
        bone_idx: 3,
        contents: 1,
        surf_flags: 0x50,
    }
}

fn placed<'a>(xmodel: &'a XModelColl<'a>, origin: [f32; 3], yaw: f32) -> StaticModel<'a> {
    StaticModel {
        xmodel: Some(xmodel),
        contents: 1,
        origin,
        inv_scaled_axis: inv_scaled_axis([0.0, yaw, 0.0], 1.0),
        absmin: [origin[0] - 12.0, origin[1] - 12.0, origin[2] - 12.0],
        absmax: [origin[0] + 12.0, origin[1] + 12.0, origin[2] + 12.0],
        ..StaticModel::default()
    }
}

#[test]
fn a_line_onto_a_collision_triangle_stops_short_of_it() {
    let surfs = [coll_surf(&FLOOR_TRI)];
    let model = XModelColl { coll_lod: 0, surfs: &surfs };
    let mut trace = Trace::new();
    assert_eq!(model.trace_line(&mut trace, [2.0, 2.0, 10.0], [2.0, 2.0, -10.0], 1), 3);
    assert_eq!(trace.fraction, (10.0 - XMODEL_TRACE_SKIN) / 20.0);
    assert_eq!(trace.normal, [0.0, 0.0, 1.0]);
    assert_eq!((trace.sflags, trace.cflags), (0x50, 1));

    // Past the edge, from below, with the wrong mask or no collision: no hit.
    let mut trace = Trace::new();
    assert_eq!(model.trace_line(&mut trace, [9.0, 9.0, 10.0], [9.0, 9.0, -10.0], 1), -1);
    assert_eq!(model.trace_line(&mut trace, [2.0, 2.0, -10.0], [2.0, 2.0, 10.0], 1), -1);
    assert_eq!(model.trace_line(&mut trace, [2.0, 2.0, 10.0], [2.0, 2.0, -10.0], 2), -1);
    let none = XModelColl { coll_lod: -1, surfs: &surfs };
    assert_eq!(none.trace_line(&mut trace, [2.0, 2.0, 10.0], [2.0, 2.0, -10.0], 1), -1);
    assert_eq!(trace.fraction, 1.0);
}

#[test]
fn a_turned_static_model_hits_in_its_own_space_and_reports_the_world_normal() {
    let surfs = [coll_surf(&WALL_TRI)];
    let xmodel = XModelColl { coll_lod: 0, surfs: &surfs };
    // Turned 90 degrees: its -x face looks down -y, across x 0 to -10.
    let model = placed(&xmodel, [50.0, 0.0, 0.0], 90.0);
    let mut trace = Trace::new();
    model.trace(7, &mut trace, [48.0, -20.0, 2.0], [48.0, 20.0, 2.0], 1);
    assert_eq!(trace.static_model, Some(7));
    assert_eq!((trace.hit_type, trace.hit_id), (trace_t6::TraceHitType::Entity, trace_t6::ENTITYNUM_WORLD));
    assert!((trace.fraction - (20.0 - XMODEL_TRACE_SKIN) / 40.0).abs() < 1e-5);
    assert!((trace.normal[1] + 1.0).abs() < 1e-5 && trace.normal[0].abs() < 1e-5);
    assert!(!model.trace_complete([48.0, -20.0, 2.0], [48.0, 20.0, 2.0], 1));
    assert!(model.trace_complete([52.0, -20.0, 2.0], [52.0, 20.0, 2.0], 1));
}

#[test]
fn point_traces_walk_the_sectors_to_the_nearest_static_model() {
    let cmodels = sector_models();
    let cm = ClipMap {
        cmodels: &cmodels,
        ..ClipMap::default()
    };
    let surfs = [coll_surf(&WALL_TRI)];
    let xmodel = XModelColl { coll_lod: 0, surfs: &surfs };
    // One each side of the root split at x = 0.
    let mut models = [placed(&xmodel, [200.0, 0.0, 0.0], 0.0), placed(&xmodel, [-100.0, 0.0, 0.0], 0.0)];
    let mut world = World::new();
    world.link_world(&cm, &Game::new(), &mut models);
    assert_eq!(world.sectors[1].children, [2, 3]);

    let mut trace = Trace::new();
    world.point_trace_static_models(&models, &mut trace, [-300.0, 2.0, 2.0], [300.0, 2.0, 2.0], 1);
    assert_eq!(trace.static_model, Some(1));
    assert!((trace.fraction - (200.0 - XMODEL_TRACE_SKIN) / 600.0).abs() < 1e-6);

    let mut trace = Trace::new();
    world.point_trace_static_models(&models, &mut trace, [0.0, 2.0, 2.0], [300.0, 2.0, 2.0], 1);
    assert_eq!(trace.static_model, Some(0));

    // A trace already stopped short of the models keeps its own hit.
    let mut trace = Trace {
        fraction: 0.25,
        ..Trace::new()
    };
    world.point_trace_static_models(&models, &mut trace, [-50.0, 2.0, 2.0], [350.0, 2.0, 2.0], 1);
    assert_eq!(trace.static_model, None);

    assert!(!world.point_trace_static_models_complete(&models, [-300.0, 2.0, 2.0], [300.0, 2.0, 2.0], 1));
    // The triangles are one-sided: from behind, the line gets through.
    assert!(world.point_trace_static_models_complete(&models, [300.0, 2.0, 2.0], [-300.0, 2.0, 2.0], 1));
    assert!(world.point_trace_static_models_complete(&models, [-300.0, 20.0, 2.0], [300.0, 20.0, 2.0], 1));
    assert!(world.point_trace_static_models_complete(&models, [-300.0, 2.0, 2.0], [300.0, 2.0, 2.0], 2));
}
