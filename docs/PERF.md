# Performance observation (Perfetto)

The design target for runtime observation is a native Perfetto trace,
`bo2mp-artifacts/runs/<uuid>/trace.pftrace` with a `manifest.json` beside it,
recorded only when `BO2MP_PERF=1`. Where a whole run spent its time, as opposed
to one frame, is covered in [`BENCH.md`](BENCH.md).

## What this tree has today

* The `perf` crate holds the hooks the engine calls: spans around work, counters
  for sampled values, and one call per notable game or render event. **No
  recorder is attached in this build.** Every hook is empty and
  `perf::recording()` returns false, so callers that gather values just for a
  counter skip that work too.
* `BO2MP_PERF_FOCUS=script_model:<id>` selects one script model whose fate is
  recorded each frame (`RenderFocus` in `render_anim`). It needs
  `BO2MP_PERF=1`, because focused facts exist only in a trace.
* `BO2MP_PERF_WINDOW=secs` shortens the observation window
  (`render_gpu/src/drawsurf/geometry_diagnostic.rs`), for tests that must end quickly.
* Console `dump [name]` writes a single `bo2mp-artifacts/dumps/*.txt`. It answers
  state-only questions and holds no history or timing.

The earlier `make bench-live`, `bench-overhead`, `scenario`, `chaos` and
`lifecycle-*` recipes, and the matching `cargo xtask bench|scenario|chaos|
net-feel|live|query|perf-overhead` commands, are not in the current Makefile or
`xtask`.

## Rules for reading a trace once a recorder exists

* Offline reports run the Trace Processor over `xtask/perfetto/*.sql` and fetch
  https://get.perfetto.dev/trace_processor when `TRACE_PROCESSOR` and PATH are
  empty.
* The in-process ring is 32 MiB. Readers refuse a trace with Perfetto errors,
  dropped or overwritten data; when the buffer runs out the read fails, so a
  half trace is never returned.
* Pin the run UUID and check the manifest every reader prints: zone, role,
  workload and git must match what is being claimed. "The newest run" is a
  convenience, not a citation.
* Spans per overlay or per draw item are forbidden. `BO2MP_PERF_FOCUS` admits one
  owner, and without it there is no owner census. Semantic events stay rare
  (`player_tick`, death, feel and the like).
