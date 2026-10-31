# `docs/`: map of the notes

Short pages (aim for 50 lines each) on what lives where and how to poke at the
running game. They stay brief on purpose, since nobody reopens a long file.

| file | covers | read it when |
|---|---|---|
| [`BUILD.md`](BUILD.md) | system packages per distro (Fedora, Debian, Arch), macOS, extras for the Windows cross build | before your first build |
| [`RUN.md`](RUN.md) | launching (`make map`, `--cmds`), why the controls stay frozen until play starts, forcing a match to start, sync by default, the verb list and its traps | before your first live run |
| [`WINDOWS.md`](WINDOWS.md) | the portable `bo2mp-launcher.exe` folder: `.env`, shortcuts into an install, writable `bo2mp-artifacts/` | building or running on Windows |
| [`DEPLOY.md`](DEPLOY.md) | the release, publish and deploy goals: which profile they build, the `.zst` named by its hash, the master picked by SHA, why provisioning is its own step | shipping a release, or asking why a player has an old version |
| [`DUO.md`](DUO.md) | driving two local players through the master: stdin console, status file, join by ID | reproducing multiplayer bugs locally |
| [`MASTER.md`](MASTER.md) | your own master over ssh from the clone: `cargo xtask master install`, a certificate you sign yourself when there is no domain, what to give players | running a relay for yourself or friends |
| [`PERF.md`](PERF.md) | the `perf` hooks, `BO2MP_PERF*`, and the rules for reading a `.pftrace` | tracing, or asking why a frame took 80 ms |
| [`BENCH.md`](BENCH.md) | `make bench`: map-load waterfall, stage `exclusive` time, frame time as a span tree, render/GPU/work counters, the run package; in-process, no trace needed | asking where a run spent its time |
| [`RENDER.md`](RENDER.md) | the nine crates of the render island, the frame path `IR -> cull -> one drawsurf list -> tess -> material -> SM3 -> wgpu`, what the GPU side owns, where the `d3d9_*` crates stop | when you touch the picture, techsets or lighting |
| [`ANIM.md`](ANIM.md) | three layers: `xanim_t6` / `dobj_t6` / `anim_t6` (facts), `xmodel_runtime` (bone tree, posing), and who picks the clip (`sim`, `render_anim`) | viewmodel, skeleton, bone hits |
| [`MAP-LOAD.md`](MAP-LOAD.md) | map load: the `session` -> `assets` -> install transaction, the walk that loads a prepared match, the lane per `ZoneGame`, the artifact cache | a zone will not load, an asset is missing, a match did not come up |
| [`ENTITIES.md`](ENTITIES.md) | the `TickInput -> sim::step -> Snapshot` funnel, the `entity_t6` taxonomy (`EntityState`, `et::*`, trajectories), which `sim` module holds what | gameplay, the netcode, replay |
| [`SIM-STEP.md`](SIM-STEP.md) | `sim::step`: one funnel for authority, prediction and replay; `StepReason`; what keeps a step deterministic | touching the step, prediction or replay |
| [`GSC-RUNTIME.md`](GSC-RUNTIME.md) | GSC to executable IR to the Bevy runtime; args, arrays and tables still share one `Runtime` | implementing gameplay or script execution |
| [`GSC-POSTFX.md`](GSC-POSTFX.md) | script vision, colour correction, blur, depth of field, bloom, fog, shake | writing or debugging GSC post effects |
| [`BOTS.md`](BOTS.md) | bots: BO2's own scripts plus the engine half, the `bot` console verbs | bot behaviour, "why is it standing there" |
| [`MOVE-T6.md`](MOVE-T6.md) | Black Ops II movement: what each piece keeps from BO2, what is not ported, the checks | changing or debugging movement |
| [`INPUT-T6.md`](INPUT-T6.md) | Black Ops II controls on the client: stance and switch-weapon holds, the pad move stretched to full speed on diagonals, the 90 degree pitch limit per frame, what is kept different and what is not ported | changing keys, mouse or pad feel |
