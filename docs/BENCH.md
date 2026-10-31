# Bench: one run, three reports, one package

Status in this worktree: the in-process recorder that this page describes
(`BO2MP_BENCH`, `make bench`, `make bench-live`, `make bench-overhead`) is not
in the `Makefile` or the crates here. A search finds `BO2MP_BENCH` only in
[`PERF.md`](PERF.md). What does exist for timing is listed first; the rest is
the report design, kept as the reference for when the recorder returns.

## What exists now

| tool | how | what it gives |
|---|---|---|
| Perfetto trace | `BO2MP_PERF=1` ([`PERF.md`](PERF.md)) | typed spans and counters in a native `.pftrace` |
| `bevy-trace` | `cargo run --profile play -p launcher --features bevy-trace` | Bevy's own `tracing` spans (render graph, `queue_submit`) as a Chrome trace |
| `span-stats` | `--features span-stats` on the launcher | every system, schedule and render-graph node added up by name into `bo2mp-artifacts/span-stats.txt` (or `BO2MP_SPAN_STATS`), flushed every 10 s, with each single run of 20 ms or more listed |
| `BO2MP_PIPELINED_RENDERING` | `0`, `false` or `off` turns it off | serialised rendering instead of the default pipelined mode |
| `BO2MP_FRAME_LATENCY` | a frame count | how far the CPU may run ahead of the GPU |

The `bevy-trace` and `span-stats` builds cost the frame they measure, so they
are never timed runs.

## Report design (recorder not present)

A recorder switched on by `BO2MP_BENCH=1` would print three independent
reports to `bo2mp-artifacts/bench/<stamp>.txt` at exit. No trace file is
needed, and any report may read MISS while the others stand. A benchmark keeps
running when the window loses focus; the window runner skips its background
60 Hz sleep.

Invocations the design assumed (a goal is always a demo name):

```bash
make bench demo0011                           # play a demo
make bench ZONE=mp_boneyard                   # a live map instead
BO2MP_BENCH_JOBS=assets make bench demo0011   # a load_jobs row per asset
BO2MP_BENCH_FRAMES=0 make bench demo0011      # skip the per-frame table
```

### 1. Map load

The waterfall from the shell command to a playable map, then the load-pool
stages, then each image plan's wait split from its decode.

* On a parallel walk a stage's own duration says little, so the table has
  `sole_open`: how much of the window that stage was the only one running. It
  does not bound what removing the stage would save.
* A stage's RSS delta covers the whole process, so deltas do not sum.
* Match audio is split by decoder, in worker time.

### 2. Frame time

Every typed span from [`PERF.md`](PERF.md) as a tree, over the frames after
the first playable one. The rows compare the first seconds with steady state
(exact percentiles), list the worst frames, and show `unclassified`.

* `unclassified` is wall time minus the union of the frame's declared
  top-level spans. Two threads inside one nanosecond count once, and the
  remainder is never clamped.
* A span is charged by intersection: a task crossing four frames shows its
  part in each and the rest as `carried in`.
* The frame clock closes and reopens at the top of `First`, before anything
  else, so each main-phase interval sits inside one frame. The frame the
  process exits in is closed at exit and its row is marked `partial`.
* Roots are declared, not read off a thread's stack. An executor may run a
  schedule's begin and end on different workers; those count as `migrated`.
  Nesting is still observed, and frames where a span outlives its parent are
  counted.
* `main/render overlap` is the share of wall time when a main schedule and the
  render thread were both inside, taken from their intervals. It is zero
  whenever rendering is serialised (`BO2MP_PIPELINED_RENDERING=0`). By default
  the render world runs one frame behind on its own thread; extraction stays a
  synchronised boundary, and `scheduling` in the manifest names the mode.
  Compare completed render cadence and `presented_state_age` together with
  main-loop wall time.

### 3. Counters

Render stages, the bodies inside `Present` and `Ui`, HUD schedule gaps, GPU
passes and per-frame work.

* A `*_schedule_interval` is the gap between two systems, which the executor
  may fill. Compare it with `hud_tess_body`; never add them.
* A counter nobody sampled prints MISS, never `0.00`.
* GPU passes overlap and resolve late, so they are never summed.

### The run package

Written next to `trace.pftrace`:

* `report.txt`: the three reports.
* `manifest.json`: what the run was. Revision and dirty-patch hash, binary and
  demo SHA-256, toolchain, adapter, window, render target and the viewport the
  passes were sized by, pools, affinity, env toggles and cache state by kind.
  An absent fact is `null`, never a default.
* `summary.json`: spans, counters and audio totals as data.
* `frames.csv`: one row per frame, with each span's overlap, every counter,
  `covered_ns`, `main_covered_ns`, `carried_in_ns`, flags and thread slots.
  GPU counters are named `delivered_*`, because a timestamp arrives frames
  after the work that queued it.
* `load_jobs.csv`: one row per load job, following `discovered`, `plan_ready`,
  `enqueued`, `started`, `decode_started`, `finished`, `joined`.
  `budget_wait_ms` is time a worker was held by the image memory threshold and
  `decode_ms` is the work. `prepared_bytes` is what the job decoded itself;
  `reused_bytes` is what it served from another job's payload with no decode,
  next to what the merge kept and dropped. Only `prepared_bytes` is work.

### Render-world handoff spans

`receive_render_world`, `extract_body` and `dispatch_render_world`. Receiving
can run worker or main-thread tasks as well as wait, so it is not CPU idle
time. Older reports' `extract_wait` folded all three phases together and cannot
be compared with the receive span directly.
