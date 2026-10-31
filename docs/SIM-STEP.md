# `sim::step`: the single entry into the world

It takes the world (mutable), the tick number, that tick's input, the frame length
in msec and the reason for the step, and it hands back the snapshot.

Defined in `crates/sim/src/carrier/mod.rs` and re-exported from the `sim`
crate root. Whatever can alter the authoritative world has to come in through
a `TickInput`, and the only thing that comes out is a `Snapshot`. There is no
other path in.

`try_step` takes the same arguments and the same route but returns
`Result<Snapshot, script::Fault>`. `step` is `try_step` that panics when a
script fails. Authority and replay steps need a script program that is loaded
and started.

## Who calls it

The server's authority, a client's prediction, and replay all call this one
function. Only the `StepReason` differs:

| reason | when it is used |
|---|---|
| `AuthorityFrame` | the server's own frame; the sole reason for which `advances_authority_world()` is true |
| `PredictNew` | a client running a tick it has not run before, ahead of the server |
| `Replay` | a client re-running a tick it already ran, after the server corrected it |

People at keyboards and bots look identical to `sim`: both show up as entries
in `TickInput.cmds`, and nothing further down needs to distinguish them.

## Design reasons

* **Explicit state.** A `SimWorld` owns its own `bevy_ecs` world. Two can be
  stepped side by side with no shared state: one in `net::authority::runtime`
  and one in `net::client::predict`.
* **Time passed in.** `tick` and `msec` are arguments rather than stored in
  the world. To run a tick again, pass it again.
* **Canonical input.** `TickInput::canonicalize()` orders commands by
  `ClientId`, and actions by `ClientId` and then request id (a stable sort).
  The same inputs therefore make the same step whatever order the network
  delivered them in.
* **Output is the return value.** The snapshot is what `step` returns, not
  something read from the world afterward. Anything missing from `Snapshot`
  cannot reach a client.

## The way back in

Two methods on `SimWorld` (in `crates/sim/src/carrier/checkpoint.rs`) bring
authoritative state back:

* `adopt_snapshot` installs an authoritative snapshot. It is only for worlds
  that run no script program, since a snapshot carries no script state; a
  world with scripts must be restored from a full `SimWorld` copy.
* `adopt_prediction_snapshot` reconciles the local client with a snapshot.

Both report a tally as an `AdoptReport` (`crates/sim/src/adopt.rs`).

Related pages: [`ENTITIES.md`](ENTITIES.md) for snapshots and
[`GSC-RUNTIME.md`](GSC-RUNTIME.md) for how scripts run.
