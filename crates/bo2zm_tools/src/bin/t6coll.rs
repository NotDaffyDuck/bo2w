//! bo2mp: what a T6 model collides with: its bounds, contents, collision
//! surfaces and physics collmaps (the boxes and brushes a script model's
//! movement and bullet traces hit). Read-only.
//! `t6coll <zone.ff> <model>...`
use std::fmt::{Debug, Display};
use std::path::PathBuf;

use asset_t6::XModelRef;

/// The zone (first word) and the models asked for (the rest).
fn split_command(mut words: impl Iterator<Item = String>) -> Option<(PathBuf, Vec<String>)> {
    let zone = PathBuf::from(words.next()?);
    Some((zone, words.collect()))
}

fn header_line(name: &str, m: &XModelRef) -> String {
    format!(
        "model {name}: mins {:?} maxs {:?} radius {} contents {:#x} coll lod {}",
        m.lo_corner, m.hi_corner, m.radius, m.contents, m.collision_detail_tier
    )
}

fn surf_line(i: usize, triangle_list: usize, bounds: [[f32; 3]; 2], bone: impl Display, contents: u32) -> String {
    let [mins, maxs] = bounds;
    format!("  coll surf {i}: {triangle_list} tris, mins {mins:?} maxs {maxs:?} bone {bone} contents {contents:#x}")
}

fn piece_line(i: usize, ty: impl Display, parts: [&dyn Debug; 4]) -> String {
    let [axes, offset, half, brush] = parts;
    format!("  collmap piece {i}: type {ty} axes {axes:?} offset {offset:?} half {half:?} brush {brush:?}")
}

/// Every line printed for one model found in the zone.
fn report(name: &str, m: &XModelRef) -> Vec<String> {
    let surfs = m
        .collision_faces
        .iter()
        .enumerate()
        .map(|(i, cs)| surf_line(i, cs.triangle_list.len(), [cs.lo_corner, cs.hi_corner], &cs.joint_position, cs.contents));
    let pieces = m
        .collmap
        .iter()
        .enumerate()
        .map(|(i, g)| piece_line(i, &g.ty, [&g.orientation, &g.offset, &g.half_lengths, &g.brush]));
    std::iter::once(header_line(name, m))
        .chain(surfs)
        .chain(std::iter::once(format!("  collmap contents {:#x}", m.collmap_contents)))
        .chain(pieces)
        .collect()
}

fn main() {
    let Some((zone, names)) = split_command(std::env::args().skip(1)) else {
        eprintln!("usage: t6coll <zone.ff> <model>...");
        return;
    };
    let capture = asset_t6::capture_zone(&zone).expect("capture");
    for name in &names {
        match capture.xmodels.iter().find(|m| &m.name == name) {
            Some(m) => report(name, m).iter().for_each(|line| println!("{line}")),
            None => println!("{name}: not in this zone"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{split_command, surf_line, piece_line};

    #[test]
    fn the_zone_comes_first_and_models_follow() {
        let (zone, names) = split_command(["z.ff", "m1", "m2"].map(str::to_owned).into_iter()).unwrap();
        assert_eq!(zone.to_str(), Some("z.ff"));
        assert_eq!(names, ["m1", "m2"]);
        assert!(split_command(std::iter::empty()).is_none());
    }

    #[test]
    fn a_collision_surface_line_shows_hex_contents() {
        assert_eq!(
            surf_line(2, 12, [[0.0, 1.0, 2.0], [3.0, 4.0, 5.5]], 7, 0x2a),
            "  coll surf 2: 12 tris, mins [0.0, 1.0, 2.0] maxs [3.0, 4.0, 5.5] bone 7 contents 0x2a"
        );
    }

    #[test]
    fn a_collmap_piece_line_prints_each_part_with_debug() {
        let line = piece_line(0, 1, [&[1, 0, 0], &[0.5f32, 0.0, 2.0], &[3.0f32, 3.0, 3.0], &Some(4u8)]);
        assert_eq!(
            line,
            "  collmap piece 0: type 1 axes [1, 0, 0] offset [0.5, 0.0, 2.0] half [3.0, 3.0, 3.0] brush Some(4)"
        );
    }
}
