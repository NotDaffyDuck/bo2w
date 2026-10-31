//! bo2mp probe: print the collision contents of BO2 models (XModel) whose
//! name holds XM_GREP: the model's contents, each collision surface's, and
//! each collision-map brush's (contents and bounds).
//! cargo run -p asset_t6 --example xmodel_coll_dump -- ZONE...
use asset_transport::T6ZoneMemory;
use asset_transport::mount_zone;
use fastfile_t6::layout::{Collmap, PhysGeomInfo, PhysGeomList, XModel, XModelCollSurf_s, cbrush_t};
use fastfile_t6::{XAssetKind, Loaded, RawRef, WalkSink, StreamRef, ZoneByteReader};

struct Sink {
    grep: String,
}

fn deref(s: &ZoneByteReader<'_>, p: RawRef, field_at: usize) -> Option<RawRef> {
    match s.address_for(p, field_at).ok() {
        Some(StreamRef::At(q)) => Some(s.follow_entry_link(q)),
        _ => None,
    }
}

fn text(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> String {
    deref(s, p, off)
        .and_then(|q| s.zero_ended_bytes(q).ok().map(|b| String::from_utf8_lossy(b).into_owned()))
        .unwrap_or_default()
}

fn v3(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> [f32; 3] {
    [0, 4, 8].map(|o| s.float_from_bytes(p, off + o).unwrap_or(f32::NAN))
}

impl WalkSink for Sink {
    fn asset_loaded(&mut self, s: &ZoneByteReader<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        if loaded.ty != XAssetKind::XModel {
            return Ok(());
        }
        let x = loaded.body;
        let name = text(s, x, XModel::name);
        if !name.to_ascii_lowercase().contains(&self.grep) {
            return Ok(());
        }
        let n_surfs = s.uint_from_bytes(x, XModel::numCollSurfs).unwrap_or(0).min(256) as usize;
        println!(
            "model {name}: contents {:#x} collSurfs {n_surfs} collmaps {}",
            s.uint_from_bytes(x, XModel::contents).unwrap_or(0),
            s.u8_at(x, XModel::numCollmaps).unwrap_or(0)
        );
        if let Some(arr) = deref(s, x, XModel::collSurfs) {
            for i in 0..n_surfs {
                let cs = arr.shifted_by(i * XModelCollSurf_s::SIZE);
                println!(
                    "  collsurf {i}: contents {:#x} surfFlags {:#x} bone {}",
                    s.uint_from_bytes(cs, XModelCollSurf_s::contents).unwrap_or(0),
                    s.uint_from_bytes(cs, XModelCollSurf_s::surfFlags).unwrap_or(0),
                    s.int_from_bytes(cs, XModelCollSurf_s::boneIdx).unwrap_or(-1)
                );
            }
        }
        let n_maps = s.u8_at(x, XModel::numCollmaps).unwrap_or(0) as usize;
        if let Some(arr) = deref(s, x, XModel::collmaps) {
            for i in 0..n_maps {
                let Some(list) = deref(s, arr.shifted_by(i * Collmap::SIZE), Collmap::geomList) else { continue };
                let count: usize = s.uint_from_bytes(list, PhysGeomList::count).unwrap_or(0).min(256) as usize;
                let Some(geoms) = deref(s, list, PhysGeomList::geoms) else { continue };
                for k in 0..count {
                    let g = geoms.shifted_by(k * PhysGeomInfo::SIZE);
                    let ty = s.int_from_bytes(g, PhysGeomInfo::r#type).unwrap_or(-1);
                    match deref(s, g, PhysGeomInfo::brush) {
                        Some(b) if ty == 2 => println!(
                            "  collmap {i} geom {k}: brush contents {:#x} mins {:?} maxs {:?}",
                            s.uint_from_bytes(b, cbrush_t::contents).unwrap_or(0),
                            v3(s, b, cbrush_t::mins),
                            v3(s, b, cbrush_t::maxs)
                        ),
                        _ => println!("  collmap {i} geom {k}: type {ty}"),
                    }
                }
            }
        }
        Ok(())
    }
}

fn main() {
    let grep = std::env::var("XM_GREP").unwrap_or_default().to_ascii_lowercase();
    for path in std::env::args().skip(1) {
        let Ok(image) = mount_zone(&path) else { continue };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::read_asset_directory(&mut stream) else { continue };
        let mut sink = Sink { grep: grep.clone() };
        println!("== {path}");
        if let Err(e) = fastfile_t6::Walker::new(&mut stream, &mut sink).walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
    }
}
