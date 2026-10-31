//! bo2zm: list every asset in Black Ops II zones by type and name.
//!
//! `t6list <zone.ff>...` prints one line per asset: zone, type, name (the
//! string most asset headers start with). Sound banks add their alias
//! count and bank file names. Nothing is written anywhere.

use std::path::PathBuf;

use asset_transport::{T6ZoneMemory, ZoneTitleKind, mount_zone};
use fastfile_t6::layout;
use fastfile_t6::{XAssetKind, Loaded, RawRef, WalkSink, Walker, StreamRef, ZoneByteReader, read_asset_directory};

fn deref(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> fastfile_t6::Result<Option<RawRef>> {
    s.address_for(p, off).map(|ptr| match ptr {
        StreamRef::At(q) => Some(s.follow_entry_link(q)),
        _ => None,
    })
}

const NAME_AT: usize = 0;

fn string_field(s: &ZoneByteReader<'_>, p: RawRef, off: usize) -> fastfile_t6::Result<String> {
    Ok(match deref(s, p, off)? {
        Some(q) => String::from_utf8_lossy(s.zero_ended_bytes(q)?).into_owned(),
        None => String::new(),
    })
}

fn fixed_string(s: &ZoneByteReader<'_>, p: RawRef, off: usize, len: usize) -> fastfile_t6::Result<String> {
    let raw = s.byte_window_from(p, off, len)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
}

/// The non-empty names, comma-separated.
fn names_joined(names: impl IntoIterator<Item = String>) -> String {
    names
        .into_iter()
        .filter(|name| name.len() > 0)
        .collect::<Vec<_>>()
        .join(",")
}

/// The zone's name for the listing: its file name without the extension.
fn zone_label(path: &std::path::Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// One asset's listing: type, name, and any extra detail.
struct AssetLine {
    kind: String,
    name: String,
    extra: String,
}

impl AssetLine {
    fn tab_line(&self, zone: &str) -> String {
        format!("{zone}\t{}\t{}{}", self.kind, self.name, self.extra)
    }
}

struct Lister {
    zone: String,
    listed: Vec<AssetLine>,
}

/// Asset types whose header does not start with a name pointer.
fn named_at_zero(ty: XAssetKind) -> bool {
    !matches!(
        ty,
        XAssetKind::Localize
    )
}

impl WalkSink for Lister {
    fn asset_loaded(&mut self, s: &ZoneByteReader<'_>, loaded: Loaded) -> fastfile_t6::Result<()> {
        let body = loaded.body;
        let name = if named_at_zero(loaded.ty) {
            string_field(s, body, NAME_AT).unwrap_or_default()
        } else if loaded.ty == XAssetKind::Localize {
            // A localized string: { value, name } - its key, then its text.
            let key = string_field(s, body, 4).unwrap_or_default();
            format!("{key} = {}", string_field(s, body, NAME_AT).unwrap_or_default())
        } else {
            String::new()
        };
        let mut extra = String::new();
        match loaded.ty {
            XAssetKind::Sound => {
                let aliases = s.uint_from_bytes(body, layout::SndBank::aliasCount)?;
                let stream_file = fixed_string(
                    s,
                    body.shifted_by(layout::SndBank::streamAssetBank),
                    layout::SndRuntimeAssetBank::filename,
                    256,
                )?;
                let load_file = fixed_string(
                    s,
                    body.shifted_by(layout::SndBank::loadAssetBank),
                    layout::SndRuntimeAssetBank::filename,
                    256,
                )?;
                let loaded_count =
                    s.uint_from_bytes(body.shifted_by(layout::SndBank::loadedAssets), layout::SndLoadedAssets::loadedCount)?;
                let entry_tally =
                    s.uint_from_bytes(body.shifted_by(layout::SndBank::loadedAssets), layout::SndLoadedAssets::entryCount)?;
                let data_size =
                    s.uint_from_bytes(body.shifted_by(layout::SndBank::loadedAssets), layout::SndLoadedAssets::dataSize)?;
                extra = format!(
                    " aliases={aliases} stream_bank='{stream_file}' load_bank='{load_file}' loaded={loaded_count}/{entry_tally} data={data_size}"
                );
            }
            XAssetKind::Weapon => {
                let display = string_field(s, body, layout::WeaponVariantDef::szDisplayName)?;
                let clip = s.int_from_bytes(body, layout::WeaponVariantDef::iClipSize)?;
                let def = deref(s, body, layout::WeaponVariantDef::weapDef)?;
                let (wname, ty, class, stow_kind) = match def {
                    Some(d) => (
                        string_field(s, d, layout::WeaponDef::szOverlayName).unwrap_or_default(),
                        s.uint_from_bytes(d, layout::WeaponDef::weapType)?,
                        s.uint_from_bytes(d, layout::WeaponDef::weapClass)?,
                        s.uint_from_bytes(d, layout::WeaponDef::inventoryType)?,
                    ),
                    None => (String::new(), 0, 0, 0),
                };
                // The variant's own perks (BG_WeaponHasPerk), two 32-bit words.
                let p0 = s.uint_from_bytes(body, layout::WeaponVariantDef::perks)?;
                let p1 = s.uint_from_bytes(body, layout::WeaponVariantDef::perks + 4)?;
                extra = format!(
                    " display='{display}' clip={clip} type={ty} class={class} inv={stow_kind} overlay='{wname}' perks={p0:#x},{p1:#x}"
                );
                // T6LIST_WEAPON=name,name: also its firing and turret numbers.
                let want = std::env::var("T6LIST_WEAPON").unwrap_or_default();
                if let Some(d) = def.filter(|_| want.split(',').any(|w| w == name)) {
                    let f = |off| s.float_from_bytes(d, off);
                    let i = |off| s.int_from_bytes(d, off);
                    let fx = |off| -> fastfile_t6::Result<String> {
                        Ok(match deref(s, d, off)? {
                            Some(p) => string_field(s, p, 0)?,
                            None => String::new(),
                        })
                    };
                    let w = layout::WeaponDef::damage;
                    let r = layout::WeaponDef::damageRange;
                    extra += &format!(
                        " fire_ms={} damage=[{} {} {} {} {} {}] ranges=[{} {} {} {} {} {}] arcs(l r t b)=[{} {} {} {}] \
                         max_range={} turn_speed=[{} {}]/[{} {}] converge(p y)=[{} {}] accuracy={} ai_spread={} \
                         player_spread={} suppress={} flash='{}' tracer='{}' fire_sound='{}'",
                        i(layout::WeaponDef::iFireTime)?,
                        i(w)?, i(w + 4)?, i(w + 8)?, i(w + 12)?, i(w + 16)?, i(w + 20)?,
                        f(r)?, f(r + 4)?, f(r + 8)?, f(r + 12)?, f(r + 16)?, f(r + 20)?,
                        f(layout::WeaponDef::leftArc)?, f(layout::WeaponDef::rightArc)?,
                        f(layout::WeaponDef::topArc)?, f(layout::WeaponDef::bottomArc)?,
                        f(layout::WeaponDef::maxRange)?,
                        f(layout::WeaponDef::minTurnSpeed)?, f(layout::WeaponDef::minTurnSpeed + 4)?,
                        f(layout::WeaponDef::maxTurnSpeed)?, f(layout::WeaponDef::maxTurnSpeed + 4)?,
                        f(layout::WeaponDef::pitchConvergenceTime)?, f(layout::WeaponDef::yawConvergenceTime)?,
                        f(layout::WeaponDef::accuracy)?, f(layout::WeaponDef::aiSpread)?,
                        f(layout::WeaponDef::playerSpread)?, f(layout::WeaponDef::suppressTime)?,
                        fx(layout::WeaponDef::worldFlashEffect)?, fx(layout::WeaponDef::tracerType)?,
                        string_field(s, d, layout::WeaponDef::fireSound)?,
                    );
                }
            }
            XAssetKind::Attachment => {
                let p0 = s.uint_from_bytes(body, layout::WeaponAttachment::perks)?;
                let p1 = s.uint_from_bytes(body, layout::WeaponAttachment::perks + 4)?;
                extra = format!(" perks={p0:#x},{p1:#x}");
            }
            // Speeds as loaded: units a second (the vehicle files' miles per
            // hour times 17.6; Buried's ghost lantern is 88 = 5 mph).
            XAssetKind::VehicleDef => {
                let f = |off| s.float_from_bytes(body, off);
                extra = format!(
                    " type={} maxSpeed={} maxSpeedVertical={} accel={} accelVertical={} rotAccel={} decelFwd={} decelSide={} decelUp={} quadRotor={} radius={}",
                    s.short_from_bytes(body, layout::VehicleDef::r#type)?,
                    f(layout::VehicleDef::maxSpeed)?,
                    f(layout::VehicleDef::maxSpeedVertical)?,
                    f(layout::VehicleDef::speed_up_rate)?,
                    f(layout::VehicleDef::accelVertical)?,
                    f(layout::VehicleDef::rotAccel)?,
                    f(layout::VehicleDef::vehHelicopterDecelerationFwd)?,
                    f(layout::VehicleDef::vehHelicopterDecelerationSide)?,
                    f(layout::VehicleDef::vehHelicopterDecelerationUp)?,
                    s.int_from_bytes(body, layout::VehicleDef::vehHelicopterQuadRotor)?,
                    f(layout::VehicleDef::radius)?,
                );
            }
            // bo2mp lane D: a vehicle's guns (its turret and gunner
            // weapons, by name).
            XAssetKind::VehicleDef => {
                let turret = string_field(s, body, layout::VehicleDef::turretWeapon).unwrap_or_default();
                let gunners = names_joined(
                    (0..4).map(|i| string_field(s, body, layout::VehicleDef::gunnerWeapon + i * 4).unwrap_or_default()),
                );
                extra = format!(" turret='{turret}' gunners='{gunners}'");
            }
            _ => {}
        }
        self.listed.push(AssetLine { kind: loaded.ty.name().to_owned(), name, extra });
        Ok(())
    }
}

/// Every asset line of one zone, or why it could not be read.
fn list_zone(path: &std::path::Path, wanted_zone: &str) -> Result<Vec<String>, String> {
    let as_text = |e: &dyn std::fmt::Display| e.to_string();
    let image = mount_zone(path).map_err(|e| as_text(&e))?;
    if image.game != ZoneTitleKind::T6 {
        return Err(String::from("not a Black Ops II zone"));
    }
    let header = image.t6_header().map_err(|e| as_text(&e))?;
    let mut memory = T6ZoneMemory::for_header(&header);
    let mut stream = memory.stream(&image.bytes).map_err(|e| as_text(&e))?;
    let table = read_asset_directory(&mut stream).map_err(|e| as_text(&e))?;
    let mut lister = Lister { listed: vec![], zone: wanted_zone.to_owned() };
    Walker::new(&mut stream, &mut lister).walk_assets(&table).map_err(|e| as_text(&e))?;
    Ok(lister.listed.iter().map(|asset| asset.tab_line(&lister.zone)).collect())
}

/// Prints one zone's lines; false when it could not be read.
fn print_zone(path: &PathBuf) -> bool {
    let zone = zone_label(path);
    match list_zone(path, &zone) {
        Ok(lines) => {
            lines.iter().for_each(|line| println!("{line}"));
            true
        }
        Err(error) => {
            eprintln!("{zone}: {error}");
            false
        }
    }
}

fn main() {
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let code = match files.as_slice() {
        [] => {
            eprintln!("usage: t6list <zone.ff>...");
            1
        }
        _ => i32::from(!files.iter().map(print_zone).fold(true, |ok, read| ok & read)),
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_joined_drops_empty_names() {
        let names = ["", "gun_a", "", "gun_b"].map(String::from);
        assert_eq!(names_joined(names), "gun_a,gun_b");
        assert_eq!(names_joined(Vec::new()), "");
    }

    #[test]
    fn a_row_is_tab_separated_with_the_extra_last() {
        let asset = AssetLine { kind: "weapon".into(), name: "ak47".into(), extra: " clip=30".into() };
        assert_eq!(asset.tab_line("mp_nuked"), "mp_nuked\tweapon\tak47 clip=30");
    }

    #[test]
    fn zone_label_is_the_file_stem() {
        assert_eq!(zone_label(std::path::Path::new("zone/all/mp_nuked.ff")), "mp_nuked");
        assert_eq!(zone_label(std::path::Path::new("")), "");
    }
}
