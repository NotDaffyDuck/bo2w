//! bo2mp probe: print BO2's old-style menus (menuDef_t) from T6 zones: every
//! menu that has an item with one of the given owner-draws (or all menus
//! whose name holds MENU_GREP), item by item: name, rect, owner-draw, text.
//! cargo run -p asset_t6 --example menu_dump -- 371,372,373,374,375 ZONE...
use asset_transport::{T6ZoneMemory, open_zone};
use fastfile_t6::layout as l;
use fastfile_t6::{AssetType, Loaded, Ptr, WalkSink, ZonePtr, ZoneStream};

struct Sink {
    draws: Vec<i32>,
    grep: String,
}

fn deref(s: &ZoneStream<'_>, p: Ptr, off: usize) -> Option<Ptr> {
    match s.ptr_at(p, off).ok()? {
        ZonePtr::Offset(q) => Some(s.resolve_alias(q)),
        _ => None,
    }
}

fn text(s: &ZoneStream<'_>, p: Ptr, off: usize) -> String {
    deref(s, p, off).and_then(|q| s.cstr_bytes(q).ok()).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

fn rect(s: &ZoneStream<'_>, p: Ptr) -> String {
    let f = |o| s.f32_at(p, o).unwrap_or(f32::NAN);
    let i = |o| s.i32_at(p, o).unwrap_or(-1);
    format!(
        "x {} y {} w {} h {} align {} {}",
        f(l::rectDef_s::x),
        f(l::rectDef_s::y),
        f(l::rectDef_s::w),
        f(l::rectDef_s::h),
        i(l::rectDef_s::horzAlign),
        i(l::rectDef_s::vertAlign)
    )
}

impl WalkSink for Sink {
    fn asset_loaded(&mut self, s: &ZoneStream<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        if loaded.ty == AssetType::Material
            && let Ok(g) = std::env::var("MAT_GREP")
        {
            let name = text(s, loaded.body, l::Material::info + l::MaterialInfo::name);
            if name.contains(&g) {
                println!("material {name:?} at {:?}", loaded.body);
            }
        }
        if loaded.ty != AssetType::Menu {
            return Ok(());
        }
        let m = loaded.body;
        let name = text(s, m, l::windowDef_t::name);
        let count = s.i32_at(m, l::menuDef_t::itemCount)?.max(0) as usize;
        let items: Vec<Ptr> = deref(s, m, l::menuDef_t::items)
            .map(|a| (0..count).filter_map(|k| deref(s, a, k * 4)).collect())
            .unwrap_or_default();
        let hit = items.iter().any(|&it| self.draws.contains(&s.i32_at(it, l::windowDef_t::ownerDraw).unwrap_or(0)))
            || (!self.grep.is_empty() && name.to_ascii_lowercase().contains(&self.grep));
        if std::env::var("MENU_LIST").is_ok() {
            let draws: Vec<i32> = items.iter().map(|&it| s.i32_at(it, l::windowDef_t::ownerDraw).unwrap_or(0)).filter(|&d| d != 0).collect();
            let texts: Vec<String> = items.iter().filter_map(|&it| deref(s, it, l::itemDef_s::typeData)).map(|t| text(s, t, l::textDef_s::text)).filter(|t| !t.is_empty()).collect();
            println!("menu {name:?} draws {draws:?} texts {texts:?}");
        }
        if !hit {
            return Ok(());
        }
        println!("menu {name:?} items {count} rect {} font {:?}", rect(s, m.at(l::windowDef_t::rect)), text(s, m, l::menuDef_t::font));
        for it in items {
            let w = |o| s.i32_at(it, o).unwrap_or(-1);
            let mut line = format!(
                "  item {:?} type {} ownerDraw {} flags {} style {} rect {} fore {:?}",
                text(s, it, l::windowDef_t::name),
                w(l::itemDef_s::r#type),
                w(l::windowDef_t::ownerDraw),
                w(l::windowDef_t::ownerDrawFlags),
                s.u8_at(it, l::windowDef_t::style).unwrap_or(0),
                rect(s, it.at(l::windowDef_t::rect)) + " client " + &rect(s, it.at(l::windowDef_t::rectClient)) + &format!(" rectExp {:?}", deref(s, it, l::itemDef_s::rectExpData).is_some()),
                (0..4).map(|k| s.f32_at(it, l::windowDef_t::foreColor + k * 4).unwrap_or(f32::NAN)).collect::<Vec<_>>(),
            );
            if let Some(rd) = deref(s, it, l::itemDef_s::rectExpData) {
                for (axis, off) in [("x", 0usize), ("y", 16), ("w", 32), ("h", 48)] {
                    let n = s.i32_at(rd, off + l::ExpressionStatement::numRpn).unwrap_or(0).clamp(0, 64) as usize;
                    let Some(arr) = deref(s, rd, off + l::ExpressionStatement::rpn) else { continue };
                    line += &format!("
    rectExp {axis}:");
                    for k in 0..n {
                        let e = arr.at(k * l::expressionRpn::SIZE);
                        let ty = s.i32_at(e, l::expressionRpn::r#type).unwrap_or(-1);
                        let a = s.i32_at(e, 4).unwrap_or(-1);
                        let b = s.i32_at(e, 8).unwrap_or(-1);
                        let bf = s.f32_at(e, 8).unwrap_or(f32::NAN);
                        let st = if ty == 0 && a == 2 { text(s, e, 8) } else { String::new() };
                        line += &format!(" ({ty} {a} {b}/{bf} {st:?})");
                    }
                }
            }
            line += &format!(" bgptr {:?}", s.ptr_at(it, l::windowDef_t::background).ok());
            if let Some(mat) = deref(s, it, l::windowDef_t::background) {
                let mut name = text(s, mat, l::Material::info + l::MaterialInfo::name);
                if !name.chars().all(|c| c.is_ascii_graphic()) {
                    name = deref(s, mat, 0).map(|m2| text(s, m2, l::Material::info + l::MaterialInfo::name)).unwrap_or_default();
                }
                line += &format!(" material {name:?}");
            }
            if let Some(anim) = deref(s, it, l::itemDef_s::animInfo) {
                let n = s.i32_at(anim, l::UIAnimInfo::animStateCount).unwrap_or(0).max(0) as usize;
                let arr = deref(s, anim, l::UIAnimInfo::animStates);
                for k in 0..n.min(16) {
                    let Some(st) = arr.and_then(|a| deref(s, a, k * 4)) else { continue };
                    line += &format!(
                        "
    state {:?} client {} fore {:?} rot {} onEvent {:?}",
                        text(s, st, l::animParamsDef_t::name),
                        rect(s, st.at(l::animParamsDef_t::rectClient)),
                        (0..4).map(|q| s.f32_at(st, l::animParamsDef_t::foreColor + q * 4).unwrap_or(f32::NAN)).collect::<Vec<_>>(),
                        s.f32_at(st, l::animParamsDef_t::rotation).unwrap_or(f32::NAN),
                        deref(s, st, l::animParamsDef_t::onEvent).is_some(),
                    );
                    let mut h = deref(s, st, l::animParamsDef_t::onEvent);
                    for _ in 0..8 {
                        let Some(hp) = h else { break };
                        line += &format!("
      on {:?}:", text(s, hp, l::GenericEventHandler::name));
                        let mut sc = deref(s, hp, l::GenericEventHandler::eventScript);
                        for _ in 0..16 {
                            let Some(sp) = sc else { break };
                            line += &format!(
                                " [type {} cond {} action {:?}]",
                                s.i32_at(sp, l::GenericEventScript::r#type).unwrap_or(-1),
                                deref(s, sp, l::GenericEventScript::condition).is_some(),
                                text(s, sp, l::GenericEventScript::action)
                            );
                            sc = deref(s, sp, l::GenericEventScript::next);
                        }
                        h = deref(s, hp, l::GenericEventHandler::next);
                    }
                }
            }
            if let Some(t) = deref(s, it, l::itemDef_s::typeData) {
                let f = |o| s.f32_at(t, o).unwrap_or(f32::NAN);
                let i = |o| s.i32_at(t, o).unwrap_or(-1);
                line += &format!(
                    "\n    text {:?} font {} align {} alignMode {} alignx {} aligny {} scale {} style {} textRect {}",
                    text(s, t, l::textDef_s::text),
                    i(l::textDef_s::fontEnum),
                    i(l::textDef_s::alignment),
                    i(l::textDef_s::textAlignMode),
                    f(l::textDef_s::textalignx),
                    f(l::textDef_s::textaligny),
                    f(l::textDef_s::textscale),
                    i(l::textDef_s::textStyle),
                    rect(s, t.at(l::textDef_s::textRect)),
                );
            }
            println!("{line}");
        }
        Ok(())
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let draws: Vec<i32> = args.next().unwrap_or_default().split(',').filter_map(|d| d.parse().ok()).collect();
    let grep = std::env::var("MENU_GREP").unwrap_or_default().to_ascii_lowercase();
    for path in args {
        let image = match open_zone(&path) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::open_asset_table(&mut stream) else { continue };
        let mut sink = Sink { draws: draws.clone(), grep: grep.clone() };
        println!("== {path}");
        let mut walker = fastfile_t6::Walker::new(&mut stream, &mut sink);
        if let Err(e) = walker.walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
    }
}
