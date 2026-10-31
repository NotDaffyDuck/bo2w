//! The last prepared map is kept, so loading the same map again copies it
//! instead of opening and decoding the zone a second time.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime};

use super::*;

/// A zone file as it was when it was read; a changed file is a different map.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ZoneStamp {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl ZoneStamp {
    /// The file as it is now; none when it cannot be looked at.
    fn of(path: &Path) -> Option<Self> {
        let stamp = |found: std::fs::Metadata| Self {
            modified: found.modified().ok(),
            len: found.len(),
            path: path.to_path_buf(),
        };
        std::fs::metadata(path).ok().map(stamp)
    }
}

struct ResidentMap {
    zone: ZoneStamp,
    common: Arc<CommonSet>,
    prepared: ReadiedSession,
    /// Its decoded images were let go once the card had them all
    /// ([`release_resident_cpu_images`]), so it can stand in for a walk only
    /// while the card still holds that walk's images.
    stripped: bool,
}

static RESIDENT: Mutex<Option<ResidentMap>> = Mutex::new(None);

fn resident() -> std::sync::MutexGuard<'static, Option<ResidentMap>> {
    RESIDENT.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// The walk whose map images the card holds now (0: none).
static ON_CARD: AtomicU64 = AtomicU64::new(0);

/// The render frontend calls this when a map's upload completes (with the
/// walk's id) and when it lets those images go (with 0).
pub fn note_gpu_resident(products_id: u64) {
    ON_CARD.store(products_id, Ordering::Relaxed);
}

/// The card has every image of walk `products_id`, so the copy kept for a
/// same-map load can let go of its decoded ones. Each is the texture's bytes
/// a second time (about 580 MB on Nuketown), which eight games on one PC
/// cannot carry. Returns how many it let go and how many bytes that freed
/// (an image the shared common set also holds stays in memory).
pub fn release_resident_cpu_images(products_id: u64) -> Option<(usize, u64)> {
    let mut slot = resident();
    let kept = slot.as_mut()?;
    if kept.stripped || kept.prepared.materials.products_id != products_id {
        return None;
    }
    let holders = Arc::strong_count(&kept.prepared.materials.population);
    let Some(population) = Arc::get_mut(&mut kept.prepared.materials.population) else {
        diag::info!(
            World,
            "resident map: walk #{products_id} keeps its decoded images (its materials are held {holders} times)"
        );
        return None;
    };
    let (mut released, mut freed) = (0, 0u64);
    for image in &mut population.images {
        let Some(decoded) = image.decoded.take() else {
            continue;
        };
        released += 1;
        if Arc::strong_count(&decoded) == 1 {
            freed += decoded.data.as_ref().map_or(0, |data| data.len() as u64);
        }
    }
    kept.stripped = true;
    Some((released, freed))
}

/// A copy of the kept map, when it is the same zone file against the same
/// common set.
fn copy_of_kept(zone: &ZoneStamp, key: &CommonKey) -> Option<ReadiedSession> {
    let common = landed_common(key)?;
    let slot = resident();
    let kept = slot.as_ref()?;
    // Without its decoded images the copy is good only for a load the card
    // already has every texture of.
    if kept.stripped && ON_CARD.load(Ordering::Relaxed) != kept.prepared.materials.products_id {
        return None;
    }
    (kept.zone == *zone && Arc::ptr_eq(&kept.common, &common)).then(|| kept.prepared.clone())
}

pub async fn read_staged_round(
    zone_archive_file: Result<PathBuf, String>,
    common_zone: Result<PathBuf, String>,
    progress: LoadCompletion,
) -> MatchLoadOutcome {
    let stamp = zone_archive_file.as_deref().ok().and_then(ZoneStamp::of);
    if let Some(stamp) = &stamp {
        let key = CommonKey::for_match(common_zone.as_deref().ok());
        let copying = Instant::now();
        if let Some(mut prepared) = copy_of_kept(stamp, &key) {
            progress.record_reused_scoped(StageId::CommonAssets, "shared common");
            progress.record_reused_scoped(StageId::MapAssets, "resident");
            progress.record_reused_scoped(StageId::Images, "map");
            let (shown, walk) = (stamp.path.display(), prepared.materials.products_id);
            let copy_ms = copying.elapsed().as_secs_f32() * 1000.0;
            let line = format!(
                "resident map: reused `{shown}` walk #{walk}, no zone opened and nothing decoded; copy took {copy_ms:.0}ms"
            );
            diag::info!(World, "{line}");
            prepared.report.push(line);
            prepared.report.extend(progress.duration_digest());
            return MatchLoadOutcome::Ready(prepared);
        }
    }

    if let Some(dropped) = resident().take() {
        diag::info!(
            World,
            "resident map: `{}` walk #{} dropped before the next walk",
            dropped.zone.path.display(),
            dropped.prepared.materials.products_id
        );
    }
    let (outcome, common) = walk_prepared_match(zone_archive_file, common_zone, progress.clone()).await;
    let mut prepared = match outcome {
        MatchLoadOutcome::Ready(walked) => walked,
        cancelled => return cancelled,
    };
    if let (Some(zone), Some(common)) = (stamp, common) {
        let keeping = Instant::now();
        let copy = prepared.clone();
        prepared.report.push(format!(
            "resident map: kept `{}` walk #{} for a same-map load ({:.0}ms)",
            zone.path.display(),
            prepared.materials.products_id,
            keeping.elapsed().as_secs_f32() * 1000.0
        ));
        *resident() = Some(ResidentMap {
            zone,
            common,
            prepared: copy,
            stripped: false,
        });
    }
    prepared.report.extend(progress.duration_digest());
    MatchLoadOutcome::Ready(prepared)
}
