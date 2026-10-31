//! bo2mp probe: print BO2's old-style menus (menuDef_t) from T6 zones: every
//! menu that has an item with one of the given owner-draws (or all menus
//! whose name holds MENU_GREP), item by item: name, rect, owner-draw, text.
//! cargo run -p asset_t6 --example menu_dump -- 371,372,373,374,375 ZONE...
use asset_transport::T6ZoneMemory;
use asset_transport::mount_zone;
use fastfile_t6::layout::{
    ExpressionStatement, GenericEventHandler, GenericEventScript, Material, MaterialInfo,
    UIAnimInfo, animParamsDef_t, expressionRpn, itemDef_s, menuDef_t, rectDef_s, textDef_s,
    windowDef_t,
};
use fastfile_t6::{XAssetKind, Loaded, RawRef, WalkSink, StreamRef, ZoneByteReader};

struct Sink {
    draws: Vec<i32>,
    grep: String,
}

fn deref(s: &ZoneByteReader<'_>, p: RawRef, field_at: usize) -> Option<RawRef> {
    match s.address_for(p, field_at).ok()? {
        StreamRef::At(q) => Some(s.follow_entry_link(q)),
        _ => None,
    }
}

fn text(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> String {
    deref(s, p, off).and_then(|q| s.zero_ended_bytes(q).ok()).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

fn rect(s: &ZoneByteReader<'_>, p: RawRef) -> String {
    let f = |o| s.float_from_bytes(p, o).unwrap_or(f32::NAN);
    let i = |o| s.int_from_bytes(p, o).unwrap_or(-1);
    format!(
        "x {} y {} w {} h {} align {} {}",
        f(rectDef_s::x),
        f(rectDef_s::y),
        f(rectDef_s::w),
        f(rectDef_s::h),
        i(rectDef_s::horzAlign),
        i(rectDef_s::vertAlign)
    )
}

impl WalkSink for Sink {
    fn asset_loaded(&mut self, s: &ZoneByteReader<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        if loaded.ty == XAssetKind::Material
            && let Ok(g) = std::env::var("MAT_GREP")
        {
            let name = text(s, loaded.body, Material::info + MaterialInfo::name);
            if name.contains(&g) {
                println!("material {name:?} at {:?}", loaded.body);
            }
        }
        if loaded.ty != XAssetKind::Menu {
            return Ok(());
        }
        let m = loaded.body;
        let name = text(s, m, windowDef_t::name);
        let count = s.int_from_bytes(m, menuDef_t::itemCount)?.max(0) as usize;
        let items: Vec<RawRef> = deref(s, m, menuDef_t::items)
            .map(|a| (0..count).filter_map(|k| deref(s, a, k * 4)).collect())
            .unwrap_or_default();
        let hit = items.iter().any(|&it| self.draws.contains(&s.int_from_bytes(it, windowDef_t::ownerDraw).unwrap_or(0)))
            || (!self.grep.is_empty() && name.to_ascii_lowercase().contains(&self.grep));
        if std::env::var("MENU_LIST").is_ok() {
            let draws: Vec<i32> = items.iter().map(|&it| s.int_from_bytes(it, windowDef_t::ownerDraw).unwrap_or(0)).filter(|&d| d != 0).collect();
            let texts: Vec<String> = items.iter().filter_map(|&it| deref(s, it, itemDef_s::typeData)).map(|t| text(s, t, textDef_s::text)).filter(|t| !t.is_empty()).collect();
            println!("menu {name:?} draws {draws:?} texts {texts:?}");
        }
        if !hit {
            return Ok(());
        }
        println!("menu {name:?} items {count} rect {} font {:?}", rect(s, m.shifted_by(windowDef_t::rect)), text(s, m, menuDef_t::font));
        for it in items {
            let w = |o| s.int_from_bytes(it, o).unwrap_or(-1);
            let mut line = format!(
                "  item {:?} type {} ownerDraw {} flags {} style {} rect {} fore {:?}",
                text(s, it, windowDef_t::name),
                w(itemDef_s::r#type),
                w(windowDef_t::ownerDraw),
                w(windowDef_t::ownerDrawFlags),
                s.u8_at(it, windowDef_t::style).unwrap_or(0),
                rect(s, it.shifted_by(windowDef_t::rect)) + " client " + &rect(s, it.shifted_by(windowDef_t::rectClient)) + &format!(" rectExp {:?}", deref(s, it, itemDef_s::rectExpData).is_some()),
                (0..4).map(|k| s.float_from_bytes(it, windowDef_t::foreColor + k * 4).unwrap_or(f32::NAN)).collect::<Vec<_>>(),
            );
            if let Some(rd) = deref(s, it, itemDef_s::rectExpData) {
                for (axis, off) in [("x", 0usize), ("y", 16), ("w", 32), ("h", 48)] {
                    let n = s.int_from_bytes(rd, off + ExpressionStatement::numRpn).unwrap_or(0).clamp(0, 64) as usize;
                    let Some(arr) = deref(s, rd, off + ExpressionStatement::rpn) else { continue };
                    line += &format!("
    rectExp {axis}:");
                    for k in 0..n {
                        let e = arr.shifted_by(k * expressionRpn::SIZE);
                        let ty = s.int_from_bytes(e, expressionRpn::r#type).unwrap_or(-1);
                        let a = s.int_from_bytes(e, 4).unwrap_or(-1);
                        let b = s.int_from_bytes(e, 8).unwrap_or(-1);
                        let bf = s.float_from_bytes(e, 8).unwrap_or(f32::NAN);
                        let st = if ty == 0 && a == 2 { text(s, e, 8) } else { String::new() };
                        line += &format!(" ({ty} {a} {b}/{bf} {st:?})");
                    }
                }
            }
            line += &format!(" bgptr {:?}", s.address_for(it, windowDef_t::background).ok());
            if let Some(mat) = deref(s, it, windowDef_t::background) {
                let mut name = text(s, mat, Material::info + MaterialInfo::name);
                if !name.chars().all(|c| c.is_ascii_graphic()) {
                    name = deref(s, mat, 0).map(|m2| text(s, m2, Material::info + MaterialInfo::name)).unwrap_or_default();
                }
                line += &format!(" material {name:?}");
            }
            if let Some(anim) = deref(s, it, itemDef_s::animInfo) {
                let n = s.int_from_bytes(anim, UIAnimInfo::animStateCount).unwrap_or(0).max(0) as usize;
                let arr = deref(s, anim, UIAnimInfo::animStates);
                for k in 0..n.min(16) {
                    let Some(st) = arr.and_then(|a| deref(s, a, k * 4)) else { continue };
                    line += &format!(
                        "
    state {:?} client {} fore {:?} rot {} onEvent {:?}",
                        text(s, st, animParamsDef_t::name),
                        rect(s, st.shifted_by(animParamsDef_t::rectClient)),
                        (0..4).map(|q| s.float_from_bytes(st, animParamsDef_t::foreColor + q * 4).unwrap_or(f32::NAN)).collect::<Vec<_>>(),
                        s.float_from_bytes(st, animParamsDef_t::rotation).unwrap_or(f32::NAN),
                        deref(s, st, animParamsDef_t::onEvent).is_some(),
                    );
                    let mut h = deref(s, st, animParamsDef_t::onEvent);
                    for _ in 0..8 {
                        let Some(hp) = h else { break };
                        line += &format!("
      on {:?}:", text(s, hp, GenericEventHandler::name));
                        let mut sc = deref(s, hp, GenericEventHandler::eventScript);
                        for _ in 0..16 {
                            let Some(sp) = sc else { break };
                            line += &format!(
                                " [type {} cond {} action {:?}]",
                                s.int_from_bytes(sp, GenericEventScript::r#type).unwrap_or(-1),
                                deref(s, sp, GenericEventScript::condition).is_some(),
                                text(s, sp, GenericEventScript::action)
                            );
                            sc = deref(s, sp, GenericEventScript::next);
                        }
                        h = deref(s, hp, GenericEventHandler::next);
                    }
                }
            }
            if let Some(t) = deref(s, it, itemDef_s::typeData) {
                let f = |o| s.float_from_bytes(t, o).unwrap_or(f32::NAN);
                let i = |o| s.int_from_bytes(t, o).unwrap_or(-1);
                line += &format!(
                    "\n    text {:?} font {} align {} alignMode {} alignx {} aligny {} scale {} style {} textRect {}",
                    text(s, t, textDef_s::text),
                    i(textDef_s::fontEnum),
                    i(textDef_s::alignment),
                    i(textDef_s::textAlignMode),
                    f(textDef_s::textalignx),
                    f(textDef_s::textaligny),
                    f(textDef_s::textscale),
                    i(textDef_s::textStyle),
                    rect(s, t.shifted_by(textDef_s::textRect)),
                );
            }
            println!("{line}");
        }
        Ok(())
    }
}

fn main() {
    let mut argv = std::env::args().skip(1);
    let draws: Vec<i32> = argv.next().unwrap_or_default().split(',').filter_map(|d| d.parse().ok()).collect();
    let grep = std::env::var("MENU_GREP").unwrap_or_default().to_ascii_lowercase();
    for path in argv {
        let image = match mount_zone(&path) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::read_asset_directory(&mut stream) else { continue };
        let mut sink = Sink { draws: draws.clone(), grep: grep.clone() };
        println!("== {path}");
        let mut walker = fastfile_t6::Walker::new(&mut stream, &mut sink);
        if let Err(e) = walker.walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
    }
}
