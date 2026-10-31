# Entities and the single funnel

## One path in, one path out

```text
human ─┐
bot    ├─→ TickInput ─→ sim::step() ─→ Snapshot ─→ client, theater or demo
replay ┘
```

The entry point is `step` in `crates/sim/src/carrier/mod.rs`, re-exported from
`crates/sim/src/lib.rs`, together with `SimWorld`, `StepReason` and `try_step`.
Headless runs, prediction and replay all go through this same function, and time
enters only as an argument (the step length in milliseconds). How the funnel is
shaped: [`SIM-STEP.md`](SIM-STEP.md).

## What an entity can be: fixed by `entity_t6`

We wrote the wiring, but the taxonomy comes from Black Ops II. `entity_t6` is plain data
and BO2's small functions over it (`no_std`, no ECS, no I/O), with BO2's field
order, numbers and trajectory maths. It does not serialize or order fields for
the network; transport belongs to `net`.

| piece | what it holds |
|---|---|
| `EntityState` | the replicated snapshot slot (`entity_state.rs`) |
| `et::*` | entity types: `PLAYER` 1, `PLAYER_CORPSE` 2, `ITEM` 3, `MISSILE` 4, plus script movers, FX, turrets, vehicles, actors |
| `Gravity::BO2` | trajectory defaults: gravity 800, low 400, moon 136 |
| `MAX_PLAYER_CORPSES` | 4 corpse slots (`corpse.rs`) |
| `MAX_GENTITIES` | 2048 here (BO2 itself has 1024, `BO2_MAX_GENTITIES`); `ENTITYNUM_NONE` is that minus 1 |
| `event_ring` | the entity-event ring, wrap `0x100` |
| `client_state` | team numbers and per-client state |
| `playerstate_t6` | player and input, the two ends of the funnel |

## Where the rest lives

* `sim/` holds the gentity, spawn, world, snapshot, damage, bullet, missile,
  item, corpse, match_state, score, rules, world_objects and identities modules.
  `ScriptModelId` is the single identity of a script model; a bare `u32` is not
  a second one.
* `frame` holds the app screen, world flag, runtime role and launch identity:
  data and set declarations, no systems.
* `session` stands a match up and tears it down (`MatchInstalled`,
  `MatchTornDown`). `net` covers the wire, deltas and prediction. `bots` is
  host-only and feeds the same `TickInput` ([`BOTS.md`](BOTS.md)).

Snapshots carry the semantics of DObj composition, never runtime trees or posed
vertices ([`ANIM.md`](ANIM.md)).
