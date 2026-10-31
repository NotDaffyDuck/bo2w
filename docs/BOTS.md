# Bots

bo2mp does not carry its own bot brain. The bots are Black Ops II's: its bot
scripts (`maps/mp/bots/_bot*`) decide goals, targets, aim, trigger, sights,
grenades, melee and dives, and the engine only carries those decisions out.

## Who does what

| where | job |
|---|---|
| `crates/console` (`feature_dispatch/bots.rs`) | parses the `bot` console verb |
| `crates/bots` | seats and joins the clients the script adds, gives them gamertags and default classes, keeps one queued command per bot |
| `crates/sim/src/t6/bots.rs` | the engine half: a test client joins through `addtestclient`, walks the best goal's route over the map's path nodes, turns its view at its difficulty's speed and presses the buttons |
| `crates/sim/src/t6/console_bots.rs` | `bot add` and `bot hold`, multiplayer only |

Each tick a bot's command replaces its client's own, the way the zombies test
player works. Bots therefore reach the step as ordinary input, the same funnel a
human uses ([`SIM-STEP.md`](SIM-STEP.md)).

## Goals

A goal is a named place (`wander`, `enemy_patrol`, `cover` ...) added by script
with a priority and a radius. The highest priority wins; among equals, the newest.

## Console verbs

* `bot add [N]` asks the game's own `spawn_bot(team)` for each bot, the call its
  lobby fill uses, so the bot picks a class and plays like any other. N is capped
  at 16 per call. An enemy goes to the side opposite the host, a friend to the
  host's side, enemies first: `bot add 1` gives one target, `bot add 2` one of each.
* `bot hold [on|off]`: while on, bots keep their view but neither move nor press
  anything. It is a flag the driver reads, because BO2 has no counterpart that
  survives a respawn.
* `bot give <id> <weapon> [attachments...]` and
  `bot tp all|<id> above <h>` / `bot tp all|<id> x y z [yaw [pitch]]`.

To judge bot behaviour, run a live match (`make map mp_raid`, see
[`RUN.md`](RUN.md)) and watch it.
