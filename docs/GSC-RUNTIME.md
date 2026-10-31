# Game scripts (GSC) in bo2mp

Two script engines live in the tree, and they do different jobs:

| engine | where | what it runs |
|---|---|---|
| `sim::script` (`lang` + `bo2`) | `crates/sim/src/script/` | the start-up stubs every Black Ops II match installs |
| `gsc_t6` | `crates/gsc_t6/` | compiled BO2 scripts (threads, waits, notifies, endons) for the Zombies work; see [`BO2ZM.md`](BO2ZM.md) |

Rust supplies the language and the engine commands; what a script decides is
the script's business.

## Loading a match's scripts

* `assets::ScriptSources` (`crates/assets/src/script_bundle.rs`) carries the
  script modules, the map's entity string and string tables. A Black Ops II map
  also brings a `T6ScriptSet`. It travels inside the prepared match to
  `session::match_apply`.
* `Program::load(resolver, roots, catalog)` reads exactly the roots it is given
  (a script cannot pull in other modules) and links every call against the
  `Catalog`. A `SourceResolver` supplies the text.
* Module names are normalised: forward slashes, lower case, no `.gsc`, and
  anything that could climb out of the script root is refused.
* Sources may be UTF-8 or single-byte; the decoder falls back to one character
  per byte.
* `T6Startup::new(resolver, gametype, map)` names the roots
  (`codescripts/delete`, `codescripts/struct`, `maps/mp/gametypes/_callbacksetup`,
  the gametype, and the map's own script when the sources have one) and the
  entries, in order: gametype `main`, map `main` (only if present), then
  `_callbacksetup::codecallback_startgametype`.
* Install runs `codescripts/struct::initstructs` itself, before the map's
  `script_struct` blocks are appended to `level.struct`.
* `Catalog::t6()` is the list of commands a start-up script may call; the
  `Realm` is `T5` or `T6`.
* The program fingerprint is a SHA-256 over the format version (3) plus, per
  module, its site and realm, its name and the SHA-256 of its bytes.
  `Runtime::program_fingerprint` exposes it.

Map entities are parsed into `LevelData` (`parse_entity_string`,
`parse_radiant_keys`, typed through `KeyType`).

## Language rules (`sim::script::lang`)

The language knows nothing about any one game. Engine commands are handed in
as a `Catalog` and executed through a `Host`.

* Values are `Int(i32)` and `Float(f32)` plus strings, vectors, objects,
  arrays and `undefined`. Int arithmetic wraps.
* Division or `%` by zero is a reported error, for ints, floats and vectors.
* Ordering or comparing values of unrelated kinds is a reported error.
* Calls nest at most 31 deep; one run may execute at most 16,000,000
  statements before it is cut off.
* A script error never stops the world. The failing expression reads as
  `undefined`, the fault goes to the host, and the function carries on. Only
  running out of budget ends a run, and that fault is sticky: the match
  stops.

## What the sim does with a script

`crates/sim/src/script/bo2/` is the Black Ops II side:

* `startup.rs` the roots and entries above.
* `natives.rs` one table of seven commands, from which both the linker's
  catalog and the runtime's `NativeRegistry` are built: `getdvar`,
  `spawnstruct`, and the player methods `spawn`, `giveweapon`,
  `setspawnweapon`, `setweaponammoclip`, `setoffhandprimaryclass`.
* `runtime.rs` the `Runtime` resource: program, heap, dvars, players, props,
  tracked missiles, queued entries, and fault state. Entry functions are
  queued and run to their end at the start of the next authority tick
  (`advance_scheduler`); nothing in this runtime waits.
* `level.rs`, `players.rs`, `presence.rs`, `weapons.rs`, the damage file,
  `controls.rs`, `match_start.rs`, `dof.rs`, `volume.rs` map script objects
  onto the engine: entities, player slots, what is shown, missiles, damage,
  action slots, match start, depth of field and trigger volumes.

Each runtime error site is reported once, as a warning (`gsc: script runtime
error: ...`) and as a `runtime_error` boundary line carrying `at=`, `function=`
and `fault=`; repeats are counted but stay quiet.

Level notifies are recorded as signals and the sim reads them after the
scheduler each tick. Two are answered by the engine itself: `EXIT_LEVEL`
(`engine:exitlevel`) and `MAP_RESTART` (`engine:map_restart`).

Scripts run only on authority frames (`StepReason::AuthorityFrame`, see
[`SIM-STEP.md`](SIM-STEP.md)). Prediction and replay steps do not run them.

## The threaded VM (`gsc_t6`)

Semantics, as measured from BO2's compiled code on the PC zones:

* `wait` rounds to 50 ms server frames (20 per second, rounded as
  `seconds * 20 + 0.5` floored) and is at least one frame;
  `waittillframeend` runs after every other thread in the frame.
* `notify` runs the threads it wakes at once, before the notifier goes on,
  with endons first. A fired endon unwinds its thread to the function that
  registered it, and the caller sees that call return `undefined`.
* Deleting an entity ends the threads that run on it or wait on it.
* Runtime errors are reported once per place, the operation yields
  `undefined`, and the thread goes on.
* Limits in `vm.rs`: 256 frames, 48 nesting levels, 5,000,000 steps per run.
* Script dvar names are hashed with djb2 over the lower-cased name
  (`gsc_t6::hash_name`).
