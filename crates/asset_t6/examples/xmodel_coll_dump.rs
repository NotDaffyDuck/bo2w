//! bo2mp probe: print the collision contents of BO2 models (XModel) whose
//! name holds XM_GREP: the model's contents, each collision surface's, and
//! each collision-map brush's (contents and bounds).
//! cargo run -p asset_t6 --example xmodel_coll_dump -- ZONE...
use asset_transport::{T6ZoneMemory, open_zone};
use fastfile_t6::layout as l;
use fastfile_t6::{AssetType, Loaded, Ptr, WalkSink, ZonePtr, ZoneStream};

struct Sink {
    grep: String,
}

fn deref(s: &ZoneStream<'_>, p: Ptr, off: usize) -> Option<Ptr> {
    match s.ptr_at(p, off).ok() {
        Some(ZonePtr::Offset(q)) => Some(s.resolve_alias(q)),
        _ => None,
    }
}

fn text(s: &ZoneStream<'_>, p: Ptr, off: usize) -> String {
    deref(s, p, off)
        .and_then(|q| s.cstr_bytes(q).ok().map(|b| String::from_utf8_lossy(b).into_owned()))
        .unwrap_or_default()
}

fn v3(s: &ZoneStream<'_>, p: Ptr, off: usize) -> [f32; 3] {
    [0, 4, 8].map(|o| s.f32_at(p, off + o).unwrap_or(f32::NAN))
}

impl WalkSink for Sink {
    fn asset_loaded(&mut self, s: &ZoneStream<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        if loaded.ty != AssetType::XModel {
            return Ok(());
        }
        let x = loaded.body;
        let name = text(s, x, l::XModel::name);
        if !name.to_ascii_lowercase().contains(&self.grep) {
            return Ok(());
        }
        let n_surfs = s.u32_at(x, l::XModel::numCollSurfs).unwrap_or(0).min(256) as usize;
        println!(
            "model {name}: contents {:#x} collSurfs {n_surfs} collmaps {}",
            s.u32_at(x, l::XModel::contents).unwrap_or(0),
            s.u8_at(x, l::XModel::numCollmaps).unwrap_or(0)
        );
        if let Some(arr) = deref(s, x, l::XModel::collSurfs) {
            for i in 0..n_surfs {
                let cs = arr.at(i * l::XModelCollSurf_s::SIZE);
                println!(
                    "  collsurf {i}: contents {:#x} surfFlags {:#x} bone {}",
                    s.u32_at(cs, l::XModelCollSurf_s::contents).unwrap_or(0),
                    s.u32_at(cs, l::XModelCollSurf_s::surfFlags).unwrap_or(0),
                    s.i32_at(cs, l::XModelCollSurf_s::boneIdx).unwrap_or(-1)
                );
            }
        }
        let n_maps = s.u8_at(x, l::XModel::numCollmaps).unwrap_or(0) as usize;
        if let Some(arr) = deref(s, x, l::XModel::collmaps) {
            for i in 0..n_maps {
                let Some(list) = deref(s, arr.at(i * l::Collmap::SIZE), l::Collmap::geomList) else { continue };
                let count = s.u32_at(list, l::PhysGeomList::count).unwrap_or(0).min(256) as usize;
                let Some(geoms) = deref(s, list, l::PhysGeomList::geoms) else { continue };
                for k in 0..count {
                    let g = geoms.at(k * l::PhysGeomInfo::SIZE);
                    let ty = s.i32_at(g, l::PhysGeomInfo::r#type).unwrap_or(-1);
                    match deref(s, g, l::PhysGeomInfo::brush) {
                        Some(b) if ty == 2 => println!(
                            "  collmap {i} geom {k}: brush contents {:#x} mins {:?} maxs {:?}",
                            s.u32_at(b, l::cbrush_t::contents).unwrap_or(0),
                            v3(s, b, l::cbrush_t::mins),
                            v3(s, b, l::cbrush_t::maxs)
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
        let Ok(image) = open_zone(&path) else { continue };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::open_asset_table(&mut stream) else { continue };
        let mut sink = Sink { grep: grep.clone() };
        println!("== {path}");
        if let Err(e) = fastfile_t6::Walker::new(&mut stream, &mut sink).walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
    }
}
