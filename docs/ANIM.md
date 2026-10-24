# Animation: what lives where

Three floors, deliberately different: the **curve facts** (pure mathematics),
the **runtime tree** (pose state) and **who picked which clip** (the game).

## The facts floor — `xanim_t6`, `dobj_t6`, `anim_t6`

`anim_iw4` was deleted 10-21. Its last piece, MW2's 10-bit packed
`legsAnim` / `torsoAnim` (9-bit index, `0x200` restart bit), gave way to
`anim_t6`'s packed value: the animation's number (table index + 1, 0 = none)
in `0x0fff` and a `0x1000` restart bit (`T6_ANIM_INDEX_MASK` /
`T6_ANIM_TOGGLE`). Curve and pose facts are in `xanim_t6`, the DObj hide bits
in `dobj_t6`.

## `xmodel_runtime` — a runtime without Bevy and without ECS

A pure evaluator, used by **both** the authority **and** the client:

* `XAnimTreeDefinition` (the immutable hierarchy) + `XAnimTreeRuntime` (state;
  leaf time normalized into `[0,1]`, `update` is the per-leaf advance);
* `DObjAnimRuntime` — `update` / `pose_request` / `compose`; DObj reuse is
  equality of `(eType, model*)`, not "always a fresh one";
* `RetainedModelCapability` — pose, `XBoneInfo`, collSurf/collTris for hitscan;
* `build_body_head_weapon_dobj` — body + head on `j_spine4` + weapon on its tag.

A missing clip **never** turns into the bind pose; additive nodes are a typed
gap, not "ordinary blending".

## Who picks the clip

* `sim` — `player_anim_script.rs`, `mantle_xanim.rs`, `anim_script_gap.rs`:
  what animation the player is in, and it travels into the snapshot as
  semantics, not as a tree;
* `assets` — `XAnimParts` → owning clips;
* `render_frontend/adapters/anim/` + `render_anim` — the viewmodel (`fpv*.rs`,
  `viewmodel_controller.rs`), third person (`third_person.rs`, `remote_body.rs`),
  kick and sway (`view_kick.rs`, `view_sway.rs`), items and projectiles.
  FPV actions come from the presented `weapAnim`, **never** from the
  keyboard; the identity of the FPV mesh is
  `weapon_iw4::get_viewmodel_weapon_index`.
  The FPV consumer reads `PreparedFpv` only, never the weapon registry: views,
  rigs and material bindings are built before Ready; an equip picks one, and a
  mandatory material that failed is a refusal naming model, surface and
  material. Remote kits (`PreparedRemoteKits`: models, hide bits, shared DObj)
  and dropped items are composed at install, not on first sight.

To watch it live: `BO2MP_PERF=1` / [`PERF.md`](PERF.md).
