use super::*;
use diag::warn;

/// True about once every five seconds while `BO2MP_T6_SMLOG` is set.
pub(super) fn script_model_log_due() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static LAST: AtomicU64 = AtomicU64::new(0);
    if !*ENABLED.get_or_init(|| std::env::var_os("BO2MP_T6_SMLOG").is_some()) {
        return false;
    }
    let epoch = std::time::UNIX_EPOCH;
    let now = std::time::SystemTime::now()
        .duration_since(epoch)
        .map_or(0, |d| d.as_secs());
    let last = LAST.load(Ordering::Relaxed);
    now >= last + 5
        && LAST
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

pub(super) fn log_script_model(names: &[&str], at: Vec3, vis: &Visibility) {
    if names
        .iter()
        .any(|n| n.contains("vending") || n.contains("packapunch"))
    {
        diag::info!(
            World,
            "t6 script model: {:?} at {:?} {:?}",
            names,
            at,
            vis
        );
    }
}

/// With `BO2MP_T6_POSE_LOG=<clip name part>`, follows the first model seen
/// playing such a clip: each frame the time past the snapshot, the clip time
/// sent and the place.
pub(super) fn log_pose_sample(
    state: &xmodel_runtime::RigPoseRecord,
    id: u32,
    clock: Option<&net::FrameClock>,
    since_snapshot: f32,
    org: [f32; 3],
) {
    static WANT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    static PICKED: AtomicU32 = AtomicU32::new(u32::MAX);
    let Some(want) = WANT.get_or_init(|| std::env::var("BO2MP_T6_POSE_LOG").ok()) else {
        return;
    };
    let leaf = state
        .tree
        .as_ref()
        .and_then(|t| t.nodes.iter().find(|n| n.clip.is_some()))
        .map(|n| {
            (
                n.clip.clone().unwrap_or_default(),
                n.state.time,
                n.state.rate,
            )
        });
    let picked = PICKED.load(Ordering::Relaxed);
    let matches_clip = leaf
        .as_ref()
        .is_some_and(|(c, _, _)| c.contains(want.as_str()));
    if picked != id {
        if picked != u32::MAX || !matches_clip {
            return;
        }
        PICKED.store(id, Ordering::Relaxed);
    }
    diag::info!(
        World,
        "t6 pose #{id}: clock {} since {since_snapshot:.4} leaf {leaf:?} drawn {:.4} at ({:.2} {:.2} {:.2})",
        clock.map_or(0, |c| c.time()),
        leaf.as_ref().map_or(0.0, |l| l.1 + l.2 * since_snapshot),
        org[0],
        org[1],
        org[2]
    );
}

pub(super) fn warn_model_missing_once(mdl: &str) {
    static MISSING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut seen) = MISSING.lock()
        && !seen.iter().any(|m| m == mdl)
    {
        seen.push(mdl.to_owned());
        warn!(World, "t6 script model not in the catalog: {}", mdl);
    }
}

/// With `BO2MP_T6_HITLOG` set, a sample of the loose pieces being considered.
pub(super) fn log_loose_piece(mdl: &str, drawn: bool, origin_in: [f32; 3]) {
    if std::env::var_os("BO2MP_T6_HITLOG").is_none() {
        return;
    }
    static SEEN: AtomicU32 = AtomicU32::new(0);
    let n = SEEN.fetch_add(1, Ordering::Relaxed);
    if n % 30 == 0 && n < 600 {
        diag::info!(
            World,
            "loose piece {} {} at {origin_in:?}",
            mdl,
            if drawn {
                "draws"
            } else {
                "has no model to draw"
            }
        );
    }
}

// ---------------------------------------------------------------------------
// Collectors
// ---------------------------------------------------------------------------
