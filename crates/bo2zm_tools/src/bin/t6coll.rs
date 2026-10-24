//! bo2mp: what a T6 model collides with: its bounds, contents, collision
//! surfaces and physics collmaps (the boxes and brushes a script model's
//! movement and bullet traces hit). Read-only.
//! `t6coll <zone.ff> <model>...`
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = args.split_first() else {
        eprintln!("usage: t6coll <zone.ff> <model>...");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
    for name in names {
        let Some(m) = c.xmodels.iter().find(|m| &m.name == name) else {
            println!("{name}: not in this zone");
            continue;
        };
        println!(
            "model {name}: mins {:?} maxs {:?} radius {} contents {:#x} coll lod {}",
            m.mins, m.maxs, m.radius, m.contents, m.coll_lod
        );
        for (i, cs) in m.coll_surfs.iter().enumerate() {
            println!(
                "  coll surf {i}: {} tris, mins {:?} maxs {:?} bone {} contents {:#x}",
                cs.tris.len(),
                cs.mins,
                cs.maxs,
                cs.bone_idx,
                cs.contents
            );
        }
        println!("  collmap contents {:#x}", m.collmap_contents);
        for (i, g) in m.collmap.iter().enumerate() {
            println!(
                "  collmap piece {i}: type {} axes {:?} offset {:?} half {:?} brush {:?}",
                g.ty, g.orientation, g.offset, g.half_lengths, g.brush
            );
        }
    }
}
