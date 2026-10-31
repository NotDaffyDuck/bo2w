# Animation: who owns what

Animation is split into three layers on purpose: the **curve facts** (pure
maths), the **runtime tree** (pose state), and **which clip the game chose**.

## Facts: `xanim_t6`, `dobj_t6`, `anim_t6`

The older Modern Warfare 2 packed `legsAnim` / `torsoAnim` (9-bit index plus a `0x200`
restart bit) is gone; the last crate holding it was deleted on 10-21. Its
replacement in `anim_t6` packs the animation number (table index + 1, 0 meaning
none) into `0x0fff` and a restart toggle into `0x1000`
(`T6_ANIM_INDEX_MASK` / `T6_ANIM_TOGGLE`). Curve and pose facts sit in
`xanim_t6`, and the DObj hide bits in `dobj_t6`.

## The runtime tree: `xanim_t6` behind `xmodel_runtime`

It is a plain evaluator with no Bevy and no ECS, used by the authority and the
client alike. `xmodel_runtime` is the one door both sides use (it re-exports the
pieces and adds the T5 destructible definitions).

* `XAnimTreeDefinition` is the immutable hierarchy; `XAnimTreeRuntime` is its
  state. Leaf time is normalised into `[0,1]` and `update` advances each leaf.
* `DObjAnimRuntime` offers `update`, `pose_request` and `compose`. A DObj is
  reused when `(eType, model*)` is unchanged, not rebuilt every time.
* `RetainedModelCapability` provides the pose, the per-bone info and the
  collSurf/collTris used for hitscan.
* `build_body_head_weapon_dobj` (in `dobj_t6`) assembles body, head on
  `j_spine4`, and weapon on its tag.

A missing clip never falls back to the bind pose, and additive nodes are
reported as a typed gap rather than treated as ordinary blending.

## Who chooses the clip

* `sim` (`t6_playeranim.rs`, `mantle_xanim/`, `t6_viewmodel.rs`) decides which
  animation a player is in; the choice reaches the snapshot as semantics, not as
  a tree.
* `assets` turns `XAnimParts` into owning clips.
* `render_anim` (`src/anim/`) covers the viewmodel (`fpv*.rs`,
  the viewmodel controller), remote bodies (`remote_body.rs`), view sway and
  kick (`view_sway.rs`, `view_kick_state/`); `src/occupancy/` covers third
  person, items and projectiles, and `render_frontend/adapters/anim/` the debris
  and dynamic entities.
  First-person actions come from the presented `weapAnim`, never from the
  keyboard, and the identity of the first-person mesh is
  `get_viewmodel_weapon_index` in `playerstate_t6`.
  The first-person consumer reads `PreparedFpv` alone and never the weapon
  registry: views, rigs and material bindings are built before Ready, an equip
  selects one, and a failed mandatory material is refused with the model,
  surface and material named. Remote kits (`PreparedRemoteKits`: models, hide
  bits, shared DObj) and dropped items are composed at install time rather than
  on first sight.

For live observation see [`PERF.md`](PERF.md).
