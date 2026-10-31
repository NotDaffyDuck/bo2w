# Rendering: crate map and frame path

The render side is a set of crates around one rule: `render_frontend` reads the
presented snapshot and plans the frame, and `render_gpu` writes the pixels and
is the only crate that depends on `wgpu`. The render side makes no gameplay
decisions, never writes `AuthorityWorld`, and decodes no assets. (Older copies
of this page said nine crates; `render_backend` no longer exists.)

| crate | what it owns |
|---|---|
| `render_frontend` | `prepare/`: world spawn, camera, visibility culling, model lighting, gfx scene (`scene/{world,spawn,cull,camera,frustum}.rs`, `prepare/gfx_scene.rs`, `worker_cmds.rs`). `adapters/`: occupancy produced from anim and FX. `assemble/`: the frame's products (`drawsurf/{list,setup,state}.rs`, `drawsurf/frame_products/`, `drawsurf/material_runtime/`, `drawsurf/tess/`) |
| `render_scene` | occupancy storage: lighting requests and results, the planes of the frustum, camera pose, `dpvs_cells`. Static light lookup and BSP cells publish on `MatchInstalled` and clear on teardown, and frame culling does not republish them. `render_anim` and `render_fx` supply the poses and the effects |
| `render_frame` | the immutable frame-product snapshot at the frontend/GPU border: `SourceRevisions`, the packed segment list, pure packing, and the `wgpu_layout` contract |
| `render_material` | pure material rules: pass setup, vertex routing, the runtime catalog, tables built ahead of time, code-constant banks |
| `render` | composition (`plugin/`), `extract.rs` plus `extract/`, and `diag/` |
| `render_gpu` | the GPU pass code under `drawsurf/` |

## Path of one frame

```text
zone -> prepare/scene/world (our own types) -> spawn (GPU)
     camera + frustum + cull, dpvs_t6 visibility
     -> assemble/drawsurf/list.rs     ONE sorted draw-surface list
       -> setup.rs   technique selection (from render_material)
       -> state.rs   pass state / blend decoded from the two state words
       -> tess/{world,smodel,xmodel,glass,sky}.rs   per-surface-type emitters
       -> material_runtime/   the sorted-material table and catalog
       -> d3d9_sm3 (shader tokens -> WGSL) -> render_frame layout -> wgpu
```

Frame products are built in `frame_products/`: open the frame's inputs, plan
the sun and spot shadow casters, build the camera, sun and spot lists (reusing
last frame's work whenever its stamp still matches), then publish them into a
ring bank that the renderer reads.

There is **one** draw-surface machine. Only the tess emitters differ per
surface type, so the world, the static models and the viewmodel are separate
emitters feeding a single list, not separate renderers.

Lighting comes from `lighting_t6` (the light grid), with
`prepare/scene/smodel_lighting.rs` and `model_lighting_atlas.rs`. Shadows are
under `assemble/drawsurf/sun_shadow/`.

A local `StaticSunAndFx` schedule overlaps the static sun partition work on
workers with FX work on the main thread. It joins before the frame products
read either result, and dynamic casters merge afterward. Sun model draws reuse
the colour geometry's index buffers.

## Handoff to the GPU

* `extract.rs` and `extract/seal.rs` seal each frame behind the frontend's
  assembly. `InstalledRenderWorld` is published when static geometry, ports or
  SMC maps change; `PublishedRenderFrame` is published every frame, and only
  when the frontend says the plans belong to one coherent frame.
* GPU-side publication lives in
  `render_gpu/src/drawsurf/frame_publish.rs`, with drawing in `draw.rs`,
  products in `products.rs` and resources in `gpu_resources.rs`.
* `d3d9_decl`, `d3d9_sm3` and `d3d9_state` carry Direct3D 9 semantics with no
  knowledge of any particular game. `session` stands the match up, and
  `assets` holds the loaded bytes.
* There are no live `r_*` dvars. Tuning goes through environment variables
  (for example `BO2MP_SINGLE_CELL=1`, which shows only the eye's own cell as a
  debugging aid) and Perfetto (`BO2MP_PERF=1`, see [`PERF.md`](PERF.md)).

Film vision presets come from the map and common fastfiles. The console
commands `visionSetNaked` and `visionReset` drive `FilmVisionView` between
them over a timed transition.
