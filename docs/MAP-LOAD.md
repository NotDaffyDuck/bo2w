# Map load: who does what

The load path has three floors:

1. **Disk**: `asset_transport` finds and opens files.
2. **Format**: `fastfile_t6` and `fastfile_t5` decode zones.
3. **Products**: the `asset_*` crates, plus `assets`, which assembles them.

`assets` reaches the disk only through the transport and knows nothing about
the zone format. The fastfile crates know the format and open no files.
Standing a match up in the app is `session`'s job, not covered here.

## The request, end to end

```text
console/ui `map`  ->  session::lifecycle    SessionSwapRequest: tear the old
                                            occupancy down, then approve the load by request id
  -> assets::match_load_walk   start_match_load puts the walk on load_pool()
                               (MatchLoadBusy while one runs); poll_match_load
                               yields PreparedMatchReady
  -> session::match_apply      apply_prepared_match -> scene + SimWorld
                               -> MatchInstalled
```

* Cancelling is `MatchLoadAbort(request_id)`. The walk returns
  `MatchLoadOutcome::Canceled` at its next checkpoint and discards nothing.
* Failure is `MapLoadFailed`. It can come from discovery or from install, and
  a request ends exactly once.

## The walk: `assets::load_prepared_match`

This is the single entry point (`crates/assets/src/session_load/`). The
composition root sees neither a fastfile crate nor an asset sink. Inside, the
work runs as parallel walks on `load_pool()`: the archive, the materials needed at startup,
the common zone and the language zones. The common zone is done before the map
because the map's techset tables need it before `build_world_draw`.

The result is `MatchLoadOutcome::Ready(PreparedMatch)`. A `PreparedMatch`
holds the scripts (`ScriptSources`), the world, one material population, fx
definitions, clip collision, the weapon registry, first-person, body,
world-weapon and projectile mesh catalogs, xanims, tracers, localized
strings, the `PreparedMap` and the penetration and hit-location tables.

* Each walk enqueues its own image plan as soon as that plan is ready, and the
  catalog keeps one row per image name.
* Two plans that resolve the same archive entry to the same payload share one
  decode. A name that two sources fill differently is an override, not a
  duplicate. `load_jobs.csv` ([`BENCH.md`](BENCH.md)) counts the two
  separately.
* The per-game adapter is `assets::lane` (`ZoneLane`, chosen by `ZoneGame`).
  Black Ops II is the only game here, so there is one lane, `T6Lane`.

## Installing it: `session::match_apply`

* Preflight (`plan.rs`) builds a `MatchInstallPlan` (mode, scripts, dvars,
  effect sets, scene conversion, drawable world). Every step that can refuse
  happens there, so nothing is published until the plan exists.
* Commit publishes the plan, boots the sim and writes `MatchInstalled`.
* Bot navigation is then prepared on `load_pool()` from a snapshot of the
  world, and admission waits for `BotNavigationReady`.

`MatchLoadOutcome::Ready` means the CPU package is done, not that rendering
can start. The loading screen also holds for:

* GPU images;
* every queued pipeline (the HUD blood film is queued as soon as its
  material exists);
* the `first_person` stage: all first-person weapon materials admitted and
  their compositions laid out (`render_anim::PreparedFpv`), plus every body,
  world-weapon, map-model and projectile material and the single-model
  DObjs (`PreparedModelMaterials`).

Remote kits and dropped-item compositions are built at install. The log line
`load ledger:` summarises each load: what was handed over or compiled fresh,
against what an earlier load left behind.

## The artifact cache: `bo2mp-artifacts/cache/<kind>/<prefix>/<key>`

A content-addressed leaf in `asset_transport::artifact_cache`, with
`cache_get`, `cache_put` and `fnv1a64`.

* The path is `cache/<kind>/<first two key chars>/<key>`. Kinds are lower-case
  ASCII plus `_`; keys are `[0-9A-Za-z._-]`, under 200 characters.
* Writes land in a temp file and are renamed, so a reader never sees half a
  blob. The first write in a process starts one background sweep that keeps
  the cache under a size budget: 4096 MiB by default, or
  `BO2MP_CACHE_BUDGET_MIB`.
* A miss is silent, since the caller computes the value anyway, and a hit must
  hold the same bytes a miss would have written. A key names every input; if
  the encoder changes, bump the format word.
* Earlier versions of this page listed live kinds `mips`, `wgsl`, `localize`,
  `nav` and `xwma_pcm` (a miss on the last one ran a batched `ffmpeg`). In
  this worktree `cache_get` and `cache_put` are re-exported but have no
  callers, so treat that list as history.
* `BO2MP_GAMES` holds the game trees; no folder name is hardcoded.
* To watch a live load: `make map mp_raid` ([`RUN.md`](RUN.md)). Stages report
  through `LoadProgress`.
