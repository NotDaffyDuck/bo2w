# Driving the live game from the console

This page covers poking a running process: loading a map, scripting a player,
grabbing dumps. For observing it, see [`PERF.md`](PERF.md). For two players
through the master, see [`make duo`](DUO.md).

## Starting a run

```bash
set -a; . ./.env; set +a          # loads BO2MP_GAMES; add DISPLAY=:0 if the session has none
make map mp_raid CMDS='spawn 0; wait 2s; quit'
cargo run --profile play -p launcher -- map t6:mp_raid --cmds '...'
```

* `make` forwards no foreign flags. The script string is `CMDS` on the
  `Makefile` and `--cmds` on the binary.
* A colon cannot be a make goal (GNU make reads it as a pattern rule), so
  `make map t6:mp_raid` fails. Write `make map ZONE=t6:mp_raid`, or call
  `cargo run` directly.
* `Makefile` goals today: `map`, `export-gltf`, `play`, `menu`, `menu-shots`,
  plus the ship targets (`launcher`, `build-windows`, `setup-windows`,
  the provisioning goal, `release`, `publish`, `deploy`, `logs`) and `clean`. Run
  `make help` for the list.
* Live goals use `[profile.play]`. For an LTO build pass `PROFILE=release`.
* Targets that older copies of this page named (`scenario`, `chaos`, `bench`,
  `bench-live`, `lifecycle-*`) are not in this `Makefile`. See
  [`BENCH.md`](BENCH.md) for the timing tools that are.

## Start-of-match freeze

The loaded game script owns the freeze. `freezecontrols` limits movement on
the authority, and snapshots carry the same limits into client prediction and
command replay. Either wait for the mode's countdown to end, or use the debug
command `force_match_start` in `--cmds`, for example after `wait world` and `spawn 0`.

**Real keys in a hidden run.** `keydown <key>` presses a keyboard key (or mouse
button) through the binds, as the window would, and holds it until `keyup <key>`;
`key <key> 0.5` is the timed form. `BO2MP_INPUT_SCRIPT=<file>` appends a file of
these commands (one a line, `#` comments) to `--cmds`. A scripted run takes input
with the window hidden or unfocused, and keeps a held key down through a focus
loss. Example: `docs/gun/ingame/keys/` (Shift sprints, never prone; Ctrl tap
crouches; Ctrl hold goes prone).

Firing, hit and decal probes must begin only after the countdown has released
the player.

## Commands wait for their own fact

No ritual `wait` is needed. A command holds the queue (a FIFO) until the fact
it depends on is true:

| command | holds until |
|---|---|
| `map` / `demo` / `play` | the swap is done and `WorldScene::spawned` is GPU-ready |
| `spawn` | `AppScreen::InGame` |
| `move` / `look` / `tp` | a matching pose has been presented |
| `force_match_start` | the match is `Playing` |
| `disconnect` | the hold has been torn down |

* `wait Ns` stays for pacing and observation windows.
* A trailing `&` refuses to wait, loudly (`map mp_rust &`).
* A trailing `!` jumps the queue. In bash write `disconnect !`, or
  history expansion bites.
* The gate times out after 120 s and aborts the rest of the script with an echo.
  Parsing lives in `crates/console/src/`.
* Hand-typed `dump`, `clip` and `screenshot` pass through any wait and leave
  the queue intact. Inside `--cmds` that takes an explicit `!`.
* Hand-typed `quit`, `exit` and `disconnect` go further: they jump every hold,
  `wait 60s` included, and release it. A `quit` written into `--cmds` does not;
  it is part of the script and takes its turn.

## Verbs

```text
map spawn class give attach name kill damage move tp look nudge press hold
release bind bot wait mark record stoprecord clip demo dump screenshot ui
disconnect quit finish_run
```

plus the debug commands `force_match_start` and `showpos`.

* `quit` leaves at once and abandons any screenshot that was queued or
  half-written. `finish_run` is the scripted ending: it waits for those files
  first.
* `disconnect` leaves the session, not just the world. It also leaves the room
  (or closes it when hosting), and works with no map installed.
* `bot` takes `add | dummy | hold | tp | give | fire`.
* `dump [name]` writes the current snapshot under `dumps/`.
* `clip` writes the last 45 s available into `clips/<ULID>/` (a demo plus a
  dump), on the host and on clients. The host records the authority; a client
  records the snapshots it received plus its own presented state.
  `demo LATEST` plays the newest one back.

## Traps

* Aim down sights with `hold +speed_throw`, not `+speed`.
* The aim binding can be `bind MOUSE2 +speed_throw` (aims only while held) or
  `bind MOUSE2 +toggleads_throw` (each press toggles). Both are under
  Options, Controls, Actions, and the choice is saved in settings.
* Bolt action (`fire_type=1`): `hold +attack` fires **one** shot, and a full
  magazine does not reload by itself. Use `press +attack` N times, then
  `press +reload`.
* `look` without `LookState` only writes `ps.viewangles`; it does not aim.
* Use (`+activate` or `+usereload`) picks up your settled C4, claymores and
  deployable gadgets when their equipment ammo slot has room.
* `give` takes a namespaced asset key, for example `give t6:weapon/an94_mp`
  (keys look like `t6:weapon/<name>`).
* Custom classes live in `bo2mp-artifacts/profile/classes.txt`, one
  tab-separated row per class, and `spawn 0` picks the first slot. Delete the
  file to get five freshly generated classes.

## Settings

* Video has a brightness slider (50 to 150 percent, neutral 100) and an FOV
  slider (65 to 120 degrees). Drag with the mouse, or press Left/Right on the
  focused slider; the value shows at its right.
* Multiplayer settings hold the player-name field: Enter edits and accepts,
  Escape cancels.
* Settings persist between launches. On Windows the file is
  `settings.cfg` inside the artifacts folder. Elsewhere it is
  `$XDG_CONFIG_HOME/bo2mp/settings.cfg`, falling back to
  `~/.config/bo2mp/settings.cfg`. `BO2MP_SETTINGS_PATH` selects a separate
  profile, which is handy for probes.
